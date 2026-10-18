// SPDX-License-Identifier: AGPL-3.0-or-later
//! Adding to the library (docs/api-v2-plan.md, V9): `POST /uploads` hands out
//! a link to `PUT` one file straight to storage; `POST /tracks` and
//! `POST /books` make items of what was put there. The size is declared up
//! front (security hardening plan, C1), so the quota or credit refuses a file
//! that cannot fit before a byte is sent, and storage refuses one larger than
//! declared. The checks are v1's; send `Idempotency-Key` with the creates.

use crate::app::AppState;
use crate::db;
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use crate::v2::{music, spoken};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
pub struct UploadRequest {
    /// `track`, `book_file`, `track_cover`, `book_cover`, `companion`; for uploaded
    /// podcasts `episode`, `episode_transcript`, `podcast_cover`, `episode_image`.
    pub kind: String,
    pub filename: String,
    pub content_type: String,
    /// Required: the file's exact size in bytes.
    pub size_bytes: i64,
}

#[derive(Serialize, ToSchema)]
pub struct Upload {
    /// Name it in `POST /tracks` or `POST /books`.
    pub upload_id: String,
    /// `PUT` the file here…
    pub url: String,
    pub method: String,
    /// …with exactly these headers: they are part of the signature.
    #[schema(value_type = Object)]
    pub headers: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}

/// A link to put one file straight into storage.
#[utoipa::path(post, path = "/uploads", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = UploadRequest,
    responses((status = 201, body = Upload),
        (status = 400, description = "Unknown kind, missing type, or a size that is not positive or over 5 GiB", body = crate::v2::problem::ProblemBody),
        (status = 402, description = "`storage_refused`: the household has no room for it", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_upload(family: FamilyContext, State(state): State<AppState>, Json(body): Json<UploadRequest>) -> Result<(StatusCode, Json<Upload>), Problem> {
    let kind = match body.kind.as_str() {
        "track" => "music_track",
        "book_file" => "audiobook_file",
        "track_cover" => "music_cover",
        "book_cover" => "audiobook_cover",
        "companion" => "companion_file",
        "episode" => "podcast_episode",
        "episode_transcript" => "podcast_transcript",
        "podcast_cover" => "podcast_cover",
        "episode_image" => "episode_image",
        other => {
            return Err(Problem::bad_request(
                "unsupported_kind",
                format!("`{other}`: one of track, book_file, track_cover, book_cover, companion, episode, episode_transcript, podcast_cover, episode_image"),
            )
                .field("/kind", "invalid", other.to_string()));
        }
    };
    let request = crate::uploads::PresignRequest {
        kind: kind.into(),
        filename: body.filename,
        content_type: body.content_type,
        size_bytes: Some(body.size_bytes),
    };
    let Json(p) = crate::uploads::presign(family, State(state), Json(request)).await?;
    Ok((
        StatusCode::CREATED,
        Json(Upload {
            upload_id: p.object_key,
            url: p.url,
            method: p.method.to_string(),
            headers: json!({ "Content-Type": p.content_type }),
            expires_at: Utc::now() + chrono::Duration::seconds(p.expires_in_secs as i64),
        }),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct NewTrack {
    pub upload_id: String,
    /// The file's own name: the title when the tags have none.
    pub filename: String,
    /// `private` (default) or `family`.
    pub visibility: Option<String>,
    /// What the tags say otherwise, when they say nothing or something wrong.
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    /// Where it goes in the own.audio folder (`Music/…`).
    pub path: Option<String>,
}

/// A track from an uploaded file.
#[utoipa::path(post, path = "/tracks", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewTrack,
    responses((status = 201, body = music::Track),
        (status = 400, description = "Nothing was put under that upload", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_track(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewTrack>) -> Result<(StatusCode, Json<music::Track>), Problem> {
    let request: crate::music::FromUploadRequest = serde_json::from_value(json!({
        "object_key": body.upload_id,
        "original_filename": body.filename,
        "visibility": body.visibility,
        "title": body.title,
        "artist": body.artist,
        "album": body.album,
        "album_artist": body.album_artist,
        "genre": body.genre,
        "track_number": body.track_number,
        "path": body.path,
    }))
    .map_err(|e| Problem::bad_request("invalid_body", e.to_string()))?;
    let viewer = family.viewer();
    let (_, Json(made)) = crate::music::create_track_from_upload(family, State(state.clone()), Json(request)).await?;
    let id: Uuid = made.track.id.parse().map_err(|e: uuid::Error| Problem::internal(anyhow::anyhow!(e)))?;
    let row = db::catalog::find_track(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    let track = music::tracks(&state, vec![row]).await?.pop().ok_or_else(Problem::not_found)?;
    Ok((StatusCode::CREATED, Json(track)))
}

#[derive(Deserialize, ToSchema)]
pub struct NewBookFile {
    pub upload_id: String,
    /// Its path inside the book's folder; decides the order of the files.
    pub relative_path: Option<String>,
    pub title: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct NewBook {
    pub title: Option<String>,
    /// `;` or ` & ` between several.
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
    /// `private` (default) or `family`.
    pub visibility: Option<String>,
    pub cover_upload_id: Option<String>,
    /// Where it goes in the own.audio folder (`Audiobooks/…`).
    pub path: Option<String>,
    pub files: Vec<NewBookFile>,
}

/// A book from uploaded files.
#[utoipa::path(post, path = "/books", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewBook,
    responses((status = 201, body = spoken::Book),
        (status = 400, description = "No files, a duplicate path, or nothing put under an upload", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_book(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewBook>) -> Result<(StatusCode, Json<spoken::Book>), Problem> {
    let files: Vec<serde_json::Value> = body
        .files
        .into_iter()
        .map(|f| json!({ "object_key": f.upload_id, "relative_path": f.relative_path, "title": f.title }))
        .collect();
    let request: crate::audiobooks::FromUploadsRequest = serde_json::from_value(json!({
        "title": body.title,
        "author": body.author,
        "narrator": body.narrator,
        "description": body.description,
        "visibility": body.visibility,
        "cover_object_key": body.cover_upload_id,
        "path": body.path,
        "files": files,
    }))
    .map_err(|e| Problem::bad_request("invalid_body", e.to_string()))?;
    let viewer = family.viewer();
    let (_, Json(made)) = crate::audiobooks::create_book_from_uploads(family, State(state.clone()), Json(request)).await?;
    let id: Uuid = made.book.id.parse().map_err(|e: uuid::Error| Problem::internal(anyhow::anyhow!(e)))?;
    let row = db::catalog::find_book(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    Ok((StatusCode::CREATED, Json(spoken::full_book(&state, row).await?)))
}

// ── Uploaded podcasts (docs/uploaded-podcasts-plan.md) ───────────────────────

const TRANSCRIPT_TYPES: &[&str] = &["text/vtt", "application/srt", "text/srt", "application/x-subrip", "text/plain", "application/json"];

#[derive(Deserialize, ToSchema)]
pub struct NewPodcast {
    pub title: String,
    /// The host or hosts, `;` or ` & ` between several.
    pub author: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    /// `private` (default) or `family`.
    pub visibility: Option<String>,
    pub cover_upload_id: Option<String>,
    /// The uploader's own id for the show: the same guid again answers the existing show.
    pub guid: Option<String>,
}

/// A podcast whose episodes its owner uploads. Never fetched from anywhere.
#[utoipa::path(post, path = "/podcasts", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewPodcast,
    responses((status = 201, body = spoken::Podcast),
        (status = 200, description = "A show with this guid exists: that show", body = spoken::Podcast),
        (status = 400, description = "No title, a bad visibility, or a cover that is not an image", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_podcast(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewPodcast>) -> Result<(StatusCode, Json<spoken::Podcast>), Problem> {
    family.require_can_upload()?;
    let title = body.title.trim();
    if title.is_empty() {
        return Err(Problem::bad_request("invalid_title", "title is required").field("/title", "required", ""));
    }
    let guid = body.guid.as_deref().map(str::trim).filter(|g| !g.is_empty());
    if guid.is_some_and(|g| g.len() > 255) {
        return Err(Problem::bad_request("invalid_guid", "guid is at most 255 characters").field("/guid", "too_long", ""));
    }
    if let Some(guid) = guid {
        if let Some(id) = db::podcasts::find_uploaded_feed_by_guid(state.db(), family.user_id, guid).await? {
            return Ok((StatusCode::OK, Json(spoken::podcast_by_id(&state, &family, id).await?)));
        }
    }
    let family_id = crate::db::access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
        .map_err(|e| Problem::bad_request("invalid_visibility", e.to_string()).field("/visibility", "invalid", ""))?;
    let cover = match body.cover_upload_id.as_deref() {
        Some(key) => {
            let adopted = crate::uploads::adopt(&state, &family, key).await?;
            if !adopted.content_type.starts_with("image/") {
                return Err(Problem::bad_request("not_an_image", format!("the cover is {}", adopted.content_type)).field("/cover_upload_id", "invalid", ""));
            }
            Some(adopted.media_object_id)
        }
        None => None,
    };
    let language = body.language.as_deref().map(str::trim).filter(|l| !l.is_empty());
    let language_base = language.and_then(crate::podcasts::base_language);
    let id = db::podcasts::insert_uploaded_feed(state.db(), db::podcasts::NewUploadedFeed {
        user_id: family.user_id,
        family_id,
        title,
        author: body.author.as_deref().map(str::trim).filter(|a| !a.is_empty()),
        description: body.description.as_deref(),
        language,
        language_base: language_base.as_deref(),
        guid,
        image_object_id: cover,
    })
    .await?;
    Ok((StatusCode::CREATED, Json(spoken::podcast_by_id(&state, &family, id).await?)))
}

#[derive(Deserialize, ToSchema)]
pub struct NewEpisode {
    /// The audio, from `POST /uploads` with kind `episode`.
    pub upload_id: String,
    /// The uploader's id for the episode, unique within the show: the same guid again
    /// answers the existing episode.
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub episode_number: Option<i32>,
    pub season_number: Option<i32>,
    /// Kind `episode_image`.
    pub image_upload_id: Option<String>,
    /// Kind `episode_transcript`: WebVTT, SRT, plain text or JSON.
    pub transcript_upload_id: Option<String>,
    /// The transcript's type when its upload did not say (`text/vtt`, `application/srt`, …).
    pub transcript_type: Option<String>,
}

/// An episode of an uploaded podcast, from uploaded files. The show's owner, or a family
/// admin for a show shared with the family.
#[utoipa::path(post, path = "/podcasts/{id}/episodes", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    params(("id" = Uuid, Path)),
    request_body = NewEpisode,
    responses((status = 201, body = spoken::Episode),
        (status = 200, description = "An episode with this guid exists: that episode", body = spoken::Episode),
        (status = 400, description = "No title or guid, audio that is not audio, or a transcript of an unknown type", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Neither the show's owner nor a family admin of a shared show", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`not_uploaded_podcast`: a feed the server fetches", body = crate::v2::problem::ProblemBody)))]
pub async fn create_episode(family: FamilyContext, State(state): State<AppState>, axum::extract::Path(id): axum::extract::Path<Uuid>, Json(body): Json<NewEpisode>) -> Result<(StatusCode, Json<spoken::Episode>), Problem> {
    family.require_can_upload()?;
    let pool = state.db();
    let feed = db::podcasts::find_feed(pool, id, family.viewer()).await?.ok_or_else(Problem::not_found)?;
    if feed.source_type != "upload" {
        return Err(Problem::new(StatusCode::CONFLICT, "not_uploaded_podcast").detail("episodes are uploaded only to an uploaded podcast; this one is fetched from its feed"));
    }
    let may = feed.user_id == family.user_id || (family.is_family_admin() && feed.family_id == Some(family.family_id));
    if !may {
        return Err(Problem::forbidden());
    }
    let guid = body.guid.trim();
    let title = body.title.trim();
    if guid.is_empty() || guid.len() > 255 {
        return Err(Problem::bad_request("invalid_guid", "guid is 1 to 255 characters").field("/guid", "invalid", ""));
    }
    if title.is_empty() {
        return Err(Problem::bad_request("invalid_title", "title is required").field("/title", "required", ""));
    }
    if let Some(existing) = db::podcasts::find_episode_id_by_guid(pool, id, guid).await? {
        return Ok((StatusCode::OK, Json(spoken::episode_by_id(&state, &family, existing).await?)));
    }

    let audio = crate::uploads::adopt(&state, &family, &body.upload_id).await?;
    if !audio.content_type.starts_with("audio/") {
        return Err(Problem::bad_request("not_audio", format!("the upload is {}", audio.content_type)).field("/upload_id", "invalid", ""));
    }
    let image = match body.image_upload_id.as_deref() {
        Some(key) => {
            let adopted = crate::uploads::adopt(&state, &family, key).await?;
            if !adopted.content_type.starts_with("image/") {
                return Err(Problem::bad_request("not_an_image", format!("the image is {}", adopted.content_type)).field("/image_upload_id", "invalid", ""));
            }
            Some(adopted.media_object_id)
        }
        None => None,
    };
    let (transcript, transcript_type) = match body.transcript_upload_id.as_deref() {
        Some(key) => {
            let adopted = crate::uploads::adopt(&state, &family, key).await?;
            let kind = body.transcript_type.clone().unwrap_or(adopted.content_type).to_ascii_lowercase();
            let kind = kind.split(';').next().unwrap_or("").trim().to_string();
            if !TRANSCRIPT_TYPES.contains(&kind.as_str()) {
                return Err(Problem::bad_request("unsupported_transcript", format!("`{kind}`: one of {}", TRANSCRIPT_TYPES.join(", "))).field("/transcript_type", "invalid", ""));
            }
            (Some(adopted.media_object_id), Some(kind))
        }
        None => (None, None),
    };
    let duration_secs = body.duration_ms.filter(|ms| *ms > 0).and_then(|ms| i32::try_from((ms + 500) / 1000).ok());

    let episode_id = db::podcasts::insert_uploaded_episode(pool, db::podcasts::NewUploadedEpisode {
        feed_id: id,
        guid,
        title,
        description: body.description.as_deref(),
        published_at: body.published_at,
        duration_secs,
        episode_number: body.episode_number,
        season_number: body.season_number,
        audio_object_id: audio.media_object_id,
        image_object_id: image,
        transcript_object_id: transcript,
        transcript_type: transcript_type.as_deref(),
    })
    .await?;
    // Where it sits in the own.audio folder, as for an episode the server stored itself.
    if let Err(e) = crate::filesync::paths::ensure_episode(pool, episode_id).await {
        tracing::warn!(%episode_id, "no path for the uploaded episode: {e:#}");
    }
    crate::podcasts::notify_new_episodes(pool, id, 1).await;
    Ok((StatusCode::CREATED, Json(spoken::episode_by_id(&state, &family, episode_id).await?)))
}
