// SPDX-License-Identifier: AGPL-3.0-or-later
//! Playback (DESIGN §6, §9; docs/api-v2-plan.md, V6).
//!
//! `POST /playback-info` takes what the client can play and answers with a
//! decision — the original (`direct`) or a smaller stream the server makes
//! (`transcode`) — and a link to it. The link is signed for one item, one
//! format and bitrate, one user and a key id, until an expiry: players that
//! cannot send a header (AVPlayer, AirPlay, Cast, cars) follow it as they
//! follow any URL, and the access token never appears in one. Each use checks
//! again that the user may still play the item.
//!
//! The link is the authorisation, not the delivery: the original is answered
//! with a redirect to the store's own short-lived URL (or the server's signed
//! `/api/v1/media` link for local storage), so the file goes straight from
//! storage to the player. A transcode streams from the server through the
//! Subsonic layer's planner and ffmpeg pool (`subsonic::transcode`).

use crate::app::AppState;
use crate::db;
use crate::db::access::Viewer;
use crate::families::FamilyContext;
use crate::subsonic::transcode;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// How long a link works (DESIGN §6: six hours).
pub const LINK_TTL_SECS: i64 = 6 * 3600;
/// The signing key in use. A key is derived from the session secret and its
/// id, so a later key can be introduced beside this one.
const KEY_ID: &str = "1";

fn key(secret: &str, kid: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"own.audio stream links v2\0");
    h.update(kid.as_bytes());
    h.update(b"\0");
    h.update(secret.as_bytes());
    h.finalize().to_vec()
}

/// What a link is for; every field is under the signature.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Grant {
    /// `track:<id>`, `book_file:<id>` or `episode:<id>`.
    item: String,
    /// `orig`, `mp3` or `aac`.
    format: String,
    kbps: u32,
    offset_secs: u32,
    user: Uuid,
    expires: i64,
}

impl Grant {
    fn message(&self, kid: &str) -> String {
        format!(
            "GET\n/api/v2/stream\n{}\n{}\n{}\n{}\n{}\n{}\n{kid}",
            self.item, self.format, self.kbps, self.offset_secs, self.user, self.expires
        )
    }

    fn sign(&self, secret: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&key(secret, KEY_ID)).expect("HMAC takes any key length");
        mac.update(self.message(KEY_ID).as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    fn verify(&self, secret: &str, kid: &str, sig: &str) -> bool {
        if kid != KEY_ID || self.expires < Utc::now().timestamp() {
            return false;
        }
        let Ok(given) = URL_SAFE_NO_PAD.decode(sig) else { return false };
        let mut mac = HmacSha256::new_from_slice(&key(secret, kid)).expect("HMAC takes any key length");
        mac.update(self.message(kid).as_bytes());
        mac.verify_slice(&given).is_ok()
    }

    fn url(&self, base: &str, secret: &str) -> String {
        format!(
            "{base}/api/v2/stream?i={}&f={}&b={}&o={}&u={}&e={}&k={KEY_ID}&s={}",
            urlencoding::encode(&self.item),
            self.format,
            self.kbps,
            self.offset_secs,
            self.user,
            self.expires,
            self.sign(secret)
        )
    }
}

/// Where the audio of an item is.
enum Source {
    /// In the server's storage: the object's key, size, type, and the length.
    Stored { key: String, size: i64, content_type: String, duration_secs: Option<i32> },
    /// A podcast episode the server does not keep: the publisher's file.
    Remote(String),
}

/// The audio of an item the viewer may play; `None` when it is not one.
async fn source(state: &AppState, viewer: Viewer, item: &str) -> Result<Option<Source>, Problem> {
    let pool = state.db();
    let Some((kind, id)) = item.split_once(':') else { return Ok(None) };
    let Ok(id) = id.parse::<Uuid>() else { return Ok(None) };
    let found: Option<(Option<Uuid>, Option<String>, Option<i32>)> = match kind {
        "track" => match db::catalog::find_track(pool, viewer, id).await? {
            Some(t) => sqlx::query_as("SELECT audio_object_id, NULL::text, duration_secs FROM music_tracks WHERE id = $1")
                .bind(t.id)
                .fetch_optional(pool)
                .await
                .map_err(anyhow::Error::from)?,
            None => None,
        },
        "book_file" => {
            let row: Option<(Uuid, Uuid, Option<i32>)> =
                sqlx::query_as("SELECT book_id, audio_object_id, duration_secs FROM audiobook_files WHERE id = $1")
                    .bind(id)
                    .fetch_optional(pool)
                    .await
                    .map_err(anyhow::Error::from)?;
            match row {
                Some((book, object, secs)) if db::catalog::find_book(pool, viewer, book).await?.is_some() => Some((Some(object), None, secs)),
                _ => None,
            }
        }
        "episode" => db::catalog::find_episode(pool, viewer, id).await?.map(|e| {
            if e.stored {
                (None, None, e.duration_secs)
            } else {
                (None, e.audio_url, e.duration_secs)
            }
        }),
        _ => None,
    };
    let Some((object, remote, duration_secs)) = found else { return Ok(None) };
    if let Some(url) = remote {
        return Ok(Some(Source::Remote(url)));
    }
    let object = match (kind, object) {
        ("episode", _) => sqlx::query_scalar::<_, Option<Uuid>>("SELECT audio_object_id FROM podcast_episodes WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(anyhow::Error::from)?,
        (_, object) => object,
    };
    let Some(object) = object else { return Ok(None) };
    let Some((key, content_type, recorded)) = db::subsonic::media_object_key_type_size(pool, object).await? else { return Ok(None) };
    // The size the upload recorded; asking the store (a round trip to R2 on the
    // hosted service) only for an object that has none.
    let size = match recorded {
        Some(size) if size > 0 => size,
        _ => state.storage().head_object(&key).await?.map_or(0, |(size, _)| size),
    };
    Ok(Some(Source::Stored { key, size, content_type, duration_secs }))
}

#[derive(Deserialize, ToSchema, Default)]
pub struct Capabilities {
    /// Content types played as they are (`audio/flac`, `audio/mpeg`, `audio/mp4`…).
    /// Empty: anything, the original is always fine.
    #[serde(default)]
    pub direct: Vec<String>,
    /// Formats the server may make instead, in order of preference: `aac`, `mp3`.
    #[serde(default)]
    pub transcode: Vec<String>,
    /// The most the connection should carry, kbps.
    pub max_bitrate_kbps: Option<u32>,
}

#[derive(Deserialize, ToSchema)]
pub struct PlaybackInfoRequest {
    /// `track`, `book_file` (one file of a book) or `episode`.
    pub kind: String,
    pub id: Uuid,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Where to start, for a transcode (the original is seeked by range).
    pub start_ms: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct PlaybackInfo {
    /// `direct` (the original) or `transcode`.
    pub decision: String,
    /// What to play: signed, no header needed, good until `expires_at`.
    pub url: String,
    pub expires_at: DateTime<Utc>,
    pub content_type: Option<String>,
    /// For a transcode, the bitrate it is made at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,
    /// Why: `client_plays_original`, `over_bitrate_limit`, `format_not_playable`,
    /// `no_transcode_target`, `transcoder_busy_original_sent`, `publisher_file`.
    pub reasons: Vec<String>,
}

/// How to play an item: the original or a stream the server makes, and a link
/// a player can follow without a header.
#[utoipa::path(post, path = "/playback-info", tag = "playback", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    request_body = PlaybackInfoRequest,
    responses((status = 200, body = PlaybackInfo), (status = 404, description = "Unknown, not visible, or not stored", body = crate::v2::problem::ProblemBody)))]
pub async fn playback_info(family: FamilyContext, State(state): State<AppState>, Json(body): Json<PlaybackInfoRequest>) -> Result<Json<PlaybackInfo>, Problem> {
    if !["track", "book_file", "episode"].contains(&body.kind.as_str()) {
        return Err(Problem::bad_request("unsupported_kind", "kind is `track`, `book_file` or `episode`").field("/kind", "invalid", body.kind.clone()));
    }
    let item = format!("{}:{}", body.kind, body.id);
    let src = source(&state, family.viewer(), &item).await?.ok_or_else(Problem::not_found)?;
    let expires = Utc::now().timestamp() + LINK_TTL_SECS;
    let expires_at = DateTime::from_timestamp(expires, 0).unwrap_or_else(Utc::now);
    let (size, content_type, duration) = match src {
        Source::Remote(url) => {
            return Ok(Json(PlaybackInfo {
                decision: "direct".into(),
                url,
                expires_at,
                content_type: None,
                bitrate_kbps: None,
                reasons: vec!["publisher_file".into()],
            }));
        }
        Source::Stored { size, content_type, duration_secs, .. } => (size, content_type, duration_secs),
    };
    let caps = &body.capabilities;
    let source_kbps = duration.filter(|d| *d > 0).map(|d| (size.max(0) as u64 * 8 / d as u64 / 1000) as u32);
    let mut reasons = Vec::new();
    let playable = caps.direct.is_empty() || caps.direct.iter().any(|c| c.eq_ignore_ascii_case(&content_type));
    let over = match (caps.max_bitrate_kbps, source_kbps) {
        (Some(limit), Some(kbps)) => kbps > limit,
        (Some(_), None) => false,
        (None, _) => false,
    };
    if !playable {
        reasons.push("format_not_playable".to_string());
    }
    if over {
        reasons.push("over_bitrate_limit".to_string());
    }
    let target = caps.transcode.iter().map(|f| f.to_ascii_lowercase()).find(|f| f == "mp3" || f == "aac");
    let base = state.config().server.base_url.as_deref().unwrap_or("").trim_end_matches('/').to_string();
    let secret = &state.config().auth.session_secret;
    let mut grant = Grant {
        item,
        format: "orig".into(),
        kbps: 0,
        offset_secs: 0,
        user: family.user_id,
        expires,
    };
    if playable && !over {
        reasons.push("client_plays_original".into());
        return Ok(Json(PlaybackInfo { decision: "direct".into(), url: grant.url(&base, secret), expires_at, content_type: Some(content_type), bitrate_kbps: None, reasons }));
    }
    let Some(format) = target else {
        reasons.push("no_transcode_target".into());
        return Ok(Json(PlaybackInfo { decision: "direct".into(), url: grant.url(&base, secret), expires_at, content_type: Some(content_type), bitrate_kbps: None, reasons }));
    };
    let plan = transcode::plan(caps.max_bitrate_kbps, Some(&format), None, if playable { source_kbps } else { None })
        .ok_or_else(|| Problem::internal(anyhow::anyhow!("no plan for {format}")))?;
    grant.format = format.clone();
    grant.kbps = plan.kbps;
    grant.offset_secs = body.start_ms.map_or(0, |ms| (ms.max(0) / 1000) as u32);
    Ok(Json(PlaybackInfo {
        decision: "transcode".into(),
        url: grant.url(&base, secret),
        expires_at,
        content_type: Some(if format == "aac" { "audio/aac" } else { "audio/mpeg" }.into()),
        bitrate_kbps: Some(plan.kbps),
        reasons,
    }))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StreamQuery {
    /// The item, `kind:id`.
    i: String,
    /// `orig`, `mp3` or `aac`.
    f: String,
    b: u32,
    o: u32,
    u: Uuid,
    /// Expiry, Unix seconds.
    e: i64,
    /// Key id.
    k: String,
    /// Signature over all of the above.
    s: String,
}

/// The audio behind a link from `POST /playback-info`. The signature is the
/// authorisation; the user must still be able to play the item.
#[utoipa::path(get, path = "/stream", tag = "playback",
    extensions(("x-permission" = json!("public"))),
    params(StreamQuery),
    responses(
        (status = 200, description = "A transcode, streamed", content_type = "audio/mpeg"),
        (status = 302, description = "The original, from storage (ranges work there)"),
        (status = 403, description = "`invalid_link`: a bad or expired signature", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "The item is gone or no longer the user's to play", body = crate::v2::problem::ProblemBody)))]
pub async fn stream(State(state): State<AppState>, Query(q): Query<StreamQuery>) -> Result<Response, Problem> {
    let grant = Grant { item: q.i, format: q.f, kbps: q.b, offset_secs: q.o, user: q.u, expires: q.e };
    if !grant.verify(&state.config().auth.session_secret, &q.k, &q.s) {
        return Err(Problem::new(StatusCode::FORBIDDEN, "invalid_link").detail("the link is not genuine or has expired: ask for playback info again"));
    }
    let pool = state.db();
    let user = db::users::find_by_id(pool, grant.user).await?.filter(|u| u.is_active).ok_or_else(Problem::not_found)?;
    let membership = db::families::ensure_membership(pool, user.id, state.hooks().one_family()).await?;
    let viewer = Viewer { user_id: user.id, family_id: membership.family_id, is_family_admin: membership.role == "family_admin" || user.role == "admin" };
    let Some(src) = source(&state, viewer, &grant.item).await? else { return Err(Problem::not_found()) };
    let key = match src {
        Source::Remote(url) => return Ok(Redirect::temporary(&url).into_response()),
        Source::Stored { key, .. } => key,
    };
    if grant.format != "orig" {
        let plan = transcode::Plan {
            format: if grant.format == "aac" { transcode::Format::Aac } else { transcode::Format::Mp3 },
            kbps: grant.kbps.clamp(32, 320),
            offset_secs: grant.offset_secs,
        };
        if let Some(opened) = state.storage().open(&key, None).await? {
            if let Some(response) = transcode::start(opened.body, plan) {
                return Ok(response);
            }
        }
        // Every transcoder busy: the original, which the client was ready to fall back to.
    }
    let url = state.storage().presigned_get(&key, 3600).await?;
    Ok(Redirect::temporary(&url).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> Grant {
        Grant { item: format!("track:{}", Uuid::nil()), format: "mp3".into(), kbps: 128, offset_secs: 0, user: Uuid::nil(), expires: Utc::now().timestamp() + 60 }
    }

    #[test]
    fn a_link_is_good_for_exactly_what_it_names() {
        let g = grant();
        let sig = g.sign("secret");
        assert!(g.verify("secret", KEY_ID, &sig));
        assert!(!g.verify("other", KEY_ID, &sig), "another server's key");
        assert!(!g.verify("secret", "2", &sig), "another key id");
        for changed in [
            Grant { item: format!("track:{}", Uuid::max()), ..g.clone() },
            Grant { format: "aac".into(), ..g.clone() },
            Grant { kbps: 320, ..g.clone() },
            Grant { user: Uuid::max(), ..g.clone() },
            Grant { expires: g.expires + 1, ..g.clone() },
        ] {
            assert!(!changed.verify("secret", KEY_ID, &sig), "{changed:?}");
        }
    }

    #[test]
    fn an_expired_link_is_refused() {
        let g = Grant { expires: Utc::now().timestamp() - 1, ..grant() };
        assert!(!g.verify("secret", KEY_ID, &g.sign("secret")));
    }
}
