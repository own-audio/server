// SPDX-License-Identifier: AGPL-3.0-or-later
//! The family's choice of where "identify this audiobook" looks
//! (`metadata::books`), offered where the edition allows it
//! (`Hooks::book_source_choice`, the open-source edition). Elsewhere it is
//! Google Books with the server's own key, and these settings only report so.

use crate::app::AppState;
use crate::auth::at_rest::Cipher;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::context::FamilyContext;
use crate::metadata::books::BookSource;
use axum::extract::{Json, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Google's API keys are 39 characters of this alphabet; the limit leaves
/// room for whatever shape a future key takes without letting anything in a
/// URL that does not belong there.
const MAX_KEY_LEN: usize = 200;

#[derive(Serialize, ToSchema)]
pub struct BookSourceResponse {
    /// Whether a family admin may change this (`features.book_source_choice`).
    pub configurable: bool,
    pub source: BookSource,
    /// The family has its own Google Books key. The key itself is never sent.
    pub google_key_set: bool,
    /// The server has a key of its own (`GOOGLE_CLOUD__BOOKS_API_KEY`), used
    /// when the family has none.
    pub server_google_key: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetBookSourceRequest {
    pub source: BookSource,
    /// A new key; `null` removes the family's key; absent keeps it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub google_books_api_key: Option<Option<String>>,
}

fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

fn cipher(state: &AppState) -> Cipher {
    Cipher::derive(&state.config().auth.session_secret, "google-books-api-key")
}

fn server_key(state: &AppState) -> Option<String> {
    state
        .config()
        .google_cloud
        .as_ref()
        .and_then(|g| g.books_api_key.as_deref())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

/// The source an identify search uses for this family, and the Google key to
/// send with it: the family's own, else the server's.
pub async fn for_identify(state: &AppState, family: &FamilyContext) -> Result<(BookSource, Option<String>), AuthError> {
    if !state.hooks().book_source_choice() {
        return Ok((BookSource::Google, server_key(state)));
    }
    let (source, sealed) = db::families::book_source(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    let source = BookSource::parse(&source).unwrap_or(BookSource::OpenLibrary);
    let key = match sealed {
        Some(sealed) => Some(cipher(state).open(&sealed).map_err(AuthError::Internal)?.0),
        None => server_key(state),
    };
    Ok((source, key))
}

async fn status(state: &AppState, family: &FamilyContext) -> Result<BookSourceResponse, AuthError> {
    let server_google_key = server_key(state).is_some();
    if !state.hooks().book_source_choice() {
        return Ok(BookSourceResponse { configurable: false, source: BookSource::Google, google_key_set: false, server_google_key });
    }
    let (source, sealed) = db::families::book_source(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(BookSourceResponse {
        configurable: true,
        source: BookSource::parse(&source).unwrap_or(BookSource::OpenLibrary),
        google_key_set: sealed.is_some(),
        server_google_key,
    })
}

/// GET /api/v1/family/settings/book-source — where identifying an audiobook
/// looks. Any member may read it; the key is never part of the answer.
#[utoipa::path(get, path = "/settings/book-source", tag = "family", security(("bearer" = [])),
    responses((status = 200, body = BookSourceResponse), (status = 401, description = "Not signed in", body = crate::http::openapi::ErrorBody)))]
pub async fn get_book_source(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<BookSourceResponse>, AuthError> {
    status(&state, &family).await.map(Json)
}

/// PUT /api/v1/family/settings/book-source — choose the source, and set or
/// remove the family's Google Books key (family_admin only, and only where
/// `features.book_source_choice`).
#[utoipa::path(put, path = "/settings/book-source", tag = "family", security(("bearer" = [])),
    request_body = SetBookSourceRequest,
    responses(
        (status = 200, body = BookSourceResponse),
        (status = 400, description = "Google without a key, or a key that cannot be one", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Not signed in", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "Not a family admin, or this server does not offer the choice", body = crate::http::openapi::ErrorBody)))]
pub async fn set_book_source(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<SetBookSourceRequest>,
) -> Result<Json<BookSourceResponse>, AuthError> {
    family.require_family_admin()?;
    if !state.hooks().book_source_choice() {
        return Err(AuthError::Forbidden);
    }

    let key = match &body.google_books_api_key {
        None => None,
        Some(None) => Some(None),
        Some(Some(key)) => {
            let key = key.trim();
            if key.is_empty() {
                Some(None)
            } else if key.len() > MAX_KEY_LEN || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(AuthError::BadRequest("that is not a Google API key".to_string()));
            } else {
                Some(Some(cipher(&state).seal(key)))
            }
        }
    };

    if body.source == BookSource::Google {
        let family_key = match &key {
            Some(new) => new.is_some(),
            None => db::families::book_source(state.db(), family.family_id)
                .await
                .map_err(AuthError::Internal)?
                .1
                .is_some(),
        };
        if !family_key && server_key(&state).is_none() {
            return Err(AuthError::BadRequest(
                "Google Books needs an API key: without one it turns this server away".to_string(),
            ));
        }
    }

    db::families::set_book_source(
        state.db(),
        family.family_id,
        body.source.as_str(),
        key.as_ref().map(|k| k.as_deref()),
    )
    .await
    .map_err(AuthError::Internal)?;

    status(&state, &family).await.map(Json)
}
