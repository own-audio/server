// SPDX-License-Identifier: AGPL-3.0-or-later
//! Artwork (DESIGN §6; docs/api-v2-plan.md, V6).
//!
//! A record's `image` is a link to `GET /images/{id}`, signed for that image
//! and an expiry rounded to the week, not for a user: artwork is of low
//! sensitivity, and links bound to a user would make every list uncacheable.
//! The client adds `w` (64, 128, 256, 512 or 1024) for the size it draws; the
//! answer is `Cache-Control: immutable`. An image is addressed by the stored
//! object, which is written once and never changes — a new cover is a new
//! object and a new link — so the effect is that of addressing by content hash
//! without waiting for the checksum job to have run.
//!
//! A size is made once and kept beside the original (`derived/…`).

use crate::app::AppState;
use crate::v2::problem::Problem;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use utoipa::IntoParams;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const SIZES: &[u32] = &[64, 128, 256, 512, 1024];
const WEEK: i64 = 7 * 86_400;

fn key(secret: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"own.audio image links v2\0");
    h.update(secret.as_bytes());
    h.finalize().to_vec()
}

fn signature(secret: &str, object: Uuid, expires: i64) -> String {
    let mut mac = HmacSha256::new_from_slice(&key(secret)).expect("HMAC takes any key length");
    mac.update(format!("{object}\n{expires}").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// The link to an image object, good for one to two weeks; the same link all
/// week, so caches keep it.
pub fn link(state: &AppState, object: Uuid) -> String {
    let now = chrono::Utc::now().timestamp();
    let expires = (now / WEEK + 2) * WEEK;
    let base = state.config().server.base_url.as_deref().unwrap_or("").trim_end_matches('/');
    format!("{base}/api/v2/images/{object}?e={expires}&s={}", signature(&state.config().auth.session_secret, object, expires))
}

/// `link` for an image the record may have.
pub fn maybe(state: &AppState, object: Option<Uuid>) -> Option<String> {
    object.map(|o| link(state, o))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImageQuery {
    /// Expiry, Unix seconds.
    e: i64,
    /// Signature.
    s: String,
    /// The longest edge wanted: 64, 128, 256, 512 or 1024; the original when absent.
    w: Option<u32>,
}

/// An image behind a link from a record's `image`.
#[utoipa::path(get, path = "/images/{id}", tag = "media",
    extensions(("x-permission" = json!("public"))),
    params(("id" = Uuid, Path), ImageQuery),
    responses(
        (status = 200, description = "The image (JPEG when scaled)", content_type = "image/*"),
        (status = 400, description = "`invalid_size`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`invalid_link`: a bad or expired signature", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "No such image", body = crate::v2::problem::ProblemBody)))]
pub async fn image(State(state): State<AppState>, Path(id): Path<Uuid>, Query(q): Query<ImageQuery>) -> Result<Response, Problem> {
    let secret = &state.config().auth.session_secret;
    let genuine = URL_SAFE_NO_PAD.decode(&q.s).ok().is_some_and(|given| {
        let mut mac = HmacSha256::new_from_slice(&key(secret)).expect("HMAC takes any key length");
        mac.update(format!("{id}\n{}", q.e).as_bytes());
        mac.verify_slice(&given).is_ok()
    });
    if !genuine || q.e < chrono::Utc::now().timestamp() {
        return Err(Problem::new(StatusCode::FORBIDDEN, "invalid_link").detail("the link is not genuine or has expired: read the record again"));
    }
    if let Some(w) = q.w {
        if !SIZES.contains(&w) {
            return Err(Problem::bad_request("invalid_size", "w is 64, 128, 256, 512 or 1024").field("w", "invalid", w.to_string()));
        }
    }
    let (object_key, content_type) = crate::db::subsonic::media_object_key_and_type(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    let store = state.storage();
    let (bytes, content_type) = match q.w {
        None => (store.get(&object_key).await?, content_type),
        Some(w) => {
            let derived = format!("derived/images/{id}/{w}.jpg");
            match store.head_object(&derived).await? {
                Some(_) => (store.get(&derived).await?, "image/jpeg".to_string()),
                None => {
                    let original = store.get(&object_key).await?;
                    let scaled = tokio::task::spawn_blocking({
                        let original = original.clone();
                        move || crate::subsonic::playback::scale_cover(&original, w)
                    })
                    .await
                    .map_err(anyhow::Error::from)?;
                    match scaled {
                        Some(jpeg) => {
                            let jpeg = Bytes::from(jpeg);
                            if let Err(e) = store.put(&derived, jpeg.clone(), "image/jpeg").await {
                                tracing::warn!(error = %e, "could not keep a scaled image");
                            }
                            (jpeg, "image/jpeg".to_string())
                        }
                        // Already that small, or not an image we scale: the original.
                        None => (original, content_type),
                    }
                }
            }
        }
    };
    Ok((
        [(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string())],
        Body::from(bytes),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_names_its_image_and_expiry() {
        let id = Uuid::nil();
        let s = signature("secret", id, 100);
        assert_eq!(s, signature("secret", id, 100));
        assert_ne!(s, signature("secret", Uuid::max(), 100));
        assert_ne!(s, signature("secret", id, 101));
        assert_ne!(s, signature("other", id, 100));
    }
}
