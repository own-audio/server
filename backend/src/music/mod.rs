// SPDX-License-Identifier: AGPL-3.0-or-later
/// Music module — tracks, playlists, streaming, cover art.
pub mod affinity;
pub mod analysis;
pub mod intent;
pub mod rules;
pub mod sequence;
pub mod models;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::db::access;
use crate::families::FamilyContext;
use crate::metadata::wikimedia;
use crate::http::multipart::read_text_field;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Json, Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use mime_guess::MimeGuess;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use std::path::Path as StdPath;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateTrackRequest {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateTrackRequest {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Absent keeps the stored value (clients predating the field send none); `null` or `""`
    /// clears it back to derived from the track artist; a value sets it.
    #[serde(default, deserialize_with = "present_or_null")]
    pub album_artist: Option<Option<String>>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
}

/// Tells an absent field (`None`, via `#[serde(default)]`) from an explicit `null` (`Some(None)`).
fn present_or_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Deserialize, ToSchema)]
pub struct MetadataSearchRequest {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    10
}

#[derive(Deserialize, ToSchema)]
pub struct SetLyricsRequest {
    /// Absent or empty clears them.
    pub lyrics: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ArtistImageInfoResponse {
    /// "Wikimedia Commons", or `None` for a picture the user supplied.
    pub source: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub source_url: Option<String>,
    pub is_user_set: bool,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ArtistImageQuery {
    pub artist: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ApplyMetadataRequest {
    pub mb_recording_id: String,
    /// Which release (album) to pull album name / track number / cover art
    /// from, if the recording appears on more than one.
    pub mb_release_id: Option<String>,
    #[serde(default = "default_true")]
    pub fetch_cover: bool,
}

fn default_true() -> bool {
    true
}

/// DEDUPLICATION_PLAN.md P3. `tier` is always `"identical"` for now — Tier 2 ("likely the same
/// recording", acoustic-fingerprint evidence) is P4; the field exists now so the client doesn't
/// need a breaking change to start rendering that tier once it exists.
#[derive(Serialize, ToSchema)]
pub struct DuplicatesResponse {
    pub groups: Vec<DuplicateGroupResponse>,
}

#[derive(Serialize, ToSchema)]
pub struct DuplicateGroupResponse {
    pub tier: String,
    pub tracks: Vec<DuplicateTrackResponse>,
}

#[derive(Serialize, ToSchema)]
pub struct DuplicateTrackResponse {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub duration_secs: Option<i32>,
    pub cover_url: Option<String>,
    pub musicbrainz_recording_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sha256: String,
    pub size_bytes: Option<i64>,
    pub content_type: String,
    /// How many playlists reference *this exact copy* — evidence for which copy to keep
    /// (DEDUPLICATION_PLAN.md's own decision: surfaced, not auto-resolved; the client doesn't
    /// re-link playlist entries to the survivor in v1).
    pub playlist_count: i64,
    pub is_starred: bool,
    /// `private` or `family` — same as `TrackResponse.visibility`. Every track this endpoint
    /// returns is scoped to the caller (`db::music::duplicate_tracks`'s own `user_id = $1`),
    /// so `is_owner` on the client's `MusicTrackDTO` conversion is always `true` regardless,
    /// but visibility itself still varies per track and isn't safe to assume.
    pub visibility: String,
}

/// What the audio file's own tags say, as opposed to what the library records.
/// The two diverge whenever a MusicBrainz match has been applied, since that
/// writes the database and leaves the file untouched.
#[derive(Serialize, ToSchema)]
pub struct FileTagsResponse {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub is_compilation: bool,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    /// Whether the file carries embedded art, without shipping the bytes.
    pub has_cover: bool,
    /// The uploaded file's own name. Often the only honest clue left: tags can
    /// be absent or wrong and a bad match overwrites the library values, but
    /// "12 Vultan's Theme.flac" still says which track this is.
    pub file_name: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TrackResponse {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Which artist this track's album is filed under — the explicit value, or the track artist
    /// without its guests. Group an album's tracks by this and `album`, never by `artist`.
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    /// Which disc of a multi-disc album, and of how many; `null` when the file does not say.
    pub disc_number: Option<i32>,
    pub disc_total: Option<i32>,
    pub duration_secs: Option<i32>,
    pub cover_url: Option<String>,
    /// Set once this track has been matched via the metadata search+apply
    /// flow — lets the client grey out "Identify" / show a matched badge.
    pub musicbrainz_recording_id: Option<String>,
    /// `private` or `family` — which folder this item lives in.
    pub visibility: String,
    /// False for family-shared items owned by someone else; those are
    /// playable but not editable by the viewer.
    pub is_owner: bool,
    /// Whose it is — for acting on a family member's item (a family shortcut).
    pub owner_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub size_bytes: Option<i64>,
    /// Nullable — the `media_checksum` job fills this in asynchronously after upload
    /// (mirror-plan B-2).
    pub sha256: Option<String>,
    /// `upload` or `folder`; a folder track's file is read-only (API revision 3).
    pub source: String,
    pub read_only: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetVisibilityRequest {
    /// `private` or `family`.
    pub visibility: String,
}

#[derive(Serialize, ToSchema)]
pub struct StreamResponse {
    pub url: String,
    pub expires_in_secs: u64,
}

#[derive(Serialize, ToSchema)]
pub struct LyricsResponse {
    /// `None` when the file has no embedded lyrics tag — distinct from a
    /// track that hasn't been checked yet, which this endpoint never
    /// returns (it always parses-and-caches before responding).
    pub lyrics: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreatePlaylistRequest {
    pub name: String,
    pub description: Option<String>,
    /// `private` (default) or `family`.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Made by the smart-playlist generator; the response then carries `generated_at`.
    #[serde(default)]
    pub generated: bool,
    /// Tracks to put in it, in order, in the same request — a generated playlist
    /// arrives with its songs. Ids the caller may not play are skipped.
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdatePlaylistRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PlaylistResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub track_count: i64,
    /// `private` or `family`.
    pub visibility: String,
    pub is_owner: bool,
    pub created_at: String,
    pub updated_at: String,
    /// When the smart-playlist generator made it; absent for an ordinary playlist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    /// When the owner chose to keep a generated playlist; absent until then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_at: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PlaylistTrackResponse {
    pub entry_id: String,
    pub position: i32,
    pub track: TrackResponse,
}

#[derive(Deserialize, ToSchema)]
pub struct AddTrackToPlaylistRequest {
    pub track_id: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderPlaylistRequest {
    pub entry_ids: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpsertProgressRequest {
    pub position_secs: f64,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ProgressResponse {
    pub track_id: String,
    pub position_secs: f64,
    pub completed: bool,
    pub updated_at: String,
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    use crate::http::openapi::map;
    OpenApiRouter::new()
        // Deduplication (DEDUPLICATION_PLAN.md) — before "Tracks" since it's conceptually a
        // different view over the same data, not a track sub-resource.
        .routes(routes!(list_duplicates))
        // Tracks
        .routes(routes!(list_tracks))
        .routes(map(routes!(upload_track), |m| m.layer(DefaultBodyLimit::disable())))
        .routes(routes!(create_track_from_upload))
        .routes(routes!(get_track, update_track, delete_track))
        .routes(routes!(stream_track))
        .routes(routes!(get_track_lyrics, set_track_lyrics))
        .routes(routes!(get_cover))
        .routes(map(routes!(upload_cover), |m| m.layer(DefaultBodyLimit::disable())))
        .routes(routes!(set_track_visibility))
        // MusicBrainz identification
        .routes(routes!(get_track_file_tags))
        .routes(routes!(rescan_track_tags))
        .routes(routes!(read_discs))
        .routes(routes!(search_track_metadata))
        .routes(routes!(apply_track_metadata))
        .routes(routes!(identify_album))
        // Track playback progress
        .routes(routes!(get_track_progress, upsert_track_progress))

        .routes(routes!(set_track_feedback, clear_track_feedback))
        .routes(routes!(list_track_feedback))
        .routes(routes!(list_starred_tracks))
        .routes(routes!(star_track, unstar_track))

        .routes(routes!(list_smart_playlists, create_smart_playlist))
        .routes(routes!(list_presets))
        .routes(routes!(resolve_rule))
        .routes(routes!(parse_intent))
        .routes(routes!(get_smart_playlist, update_smart_playlist, delete_smart_playlist))
        .routes(routes!(resolve_smart_playlist))
        .routes(routes!(freeze_smart_playlist))
        // Playlists
        .routes(routes!(list_playlists, create_playlist))
        .routes(routes!(get_playlist, update_playlist, delete_playlist))
        .routes(routes!(keep_playlist))
        .routes(routes!(playlist_audience))
        .routes(routes!(share_playlist))
        .routes(routes!(list_playlist_tracks, add_to_playlist))
        .routes(routes!(remove_from_playlist))
        .routes(routes!(reorder_playlist))
        .routes(routes!(set_playlist_visibility))
        .routes(routes!(get_playlist_cover))
        .routes(map(routes!(upload_playlist_cover), |m| m.layer(DefaultBodyLimit::disable())))
        // Grouped browsing — mobile clients need artist/album navigation
        // without reimplementing the grouping over a flat track list.
        .routes(routes!(list_artists))
        .routes(routes!(get_artist_image, delete_artist_image))
        .routes(map(routes!(upload_artist_image), |m| m.layer(DefaultBodyLimit::disable())))
        .routes(routes!(get_artist_image_info))
        .routes(routes!(list_albums))
        .routes(routes!(list_genres))
}

// ── Grouped browsing ──────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct ArtistSummary {
    pub artist: String,
    pub album_count: i64,
    pub track_count: i64,
}

#[derive(Serialize, ToSchema)]
pub struct AlbumSummary {
    pub artist: String,
    pub album: String,
    pub track_count: i64,
    pub duration_secs: Option<i64>,
    pub cover_url: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct GenreSummary {
    pub genre: String,
    pub track_count: i64,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AlbumsQuery {
    /// Restrict to one artist; omit for the whole library.
    #[serde(default)]
    pub artist: Option<String>,
}

/// GET /api/v1/music/artists
/// Artists the caller can see.
#[utoipa::path(get, path = "/artists", tag = "music", security(("bearer" = [])),
    responses((status = 200, body = Vec<ArtistSummary>)))]
async fn list_artists(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<ArtistSummary>>, AuthError> {
    let rows = db::music::list_artists(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(artist, album_count, track_count)| ArtistSummary {
                artist,
                album_count,
                track_count,
            })
            .collect(),
    ))
}

/// GET /api/v1/music/artists/image?artist=… — the artist's photo.
///
/// Fetched from a public source on first request and cached, the same
/// lazy shape `get_lyrics` uses: an artist has no upload step where this could
/// have been done eagerly, and fetching every artist in a library up front
/// would be a lot of network for pictures nobody may look at.
///
/// The artist arrives as a query parameter rather than a path segment because
/// an artist here *is* its name — free text that routinely contains slashes
/// ("AC/DC"), dots and unicode.
///
/// A miss is cached too. Most libraries hold at least one name no catalogue
/// knows, and without a negative entry every grid render would re-ask the
/// network about it forever.
#[utoipa::path(get, path = "/artists/image", tag = "music", security(("bearer" = [])),
    params(ArtistImageQuery),
    responses((status = 200, description = "The image bytes", content_type = "image/*"), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, description = "No picture known for this artist", body = crate::http::openapi::ErrorBody)))]
async fn get_artist_image(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<ArtistImageQuery>,
) -> Result<Response, AuthError> {
    let artist = params.artist.trim();
    if artist.is_empty() {
        return Err(AuthError::BadRequest("artist is required".to_string()));
    }

    if let Some(cached) = db::music::find_artist_image(state.db(), family.user_id, artist)
        .await
        .map_err(AuthError::Internal)?
    {
        // A row with no object is the cached miss — answer it without touching
        // the network again.
        let Some((object_key, content_type)) = cached else {
            return Err(AuthError::ItemNotFound);
        };
        let bytes = state
            .storage()
            .get(&object_key)
            .await
            .map_err(AuthError::Internal)?;
        return Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response());
    }

    // The MusicBrainz artist id, if any of this artist's tracks have been
    // identified — an exact identity, and far better than matching on a name.
    // Absent is normal and handled: the lookup falls back to an exact-label
    // match constrained to entities that carry a MusicBrainz artist id at all.
    let mb_artist_id = db::music::find_artist_mbid(state.db(), family.user_id, artist)
        .await
        .map_err(AuthError::Internal)?;

    let mut image = match wikimedia::resolve(artist, mb_artist_id.as_deref()).await
    {
        Ok(image) => image,
        // Remember the miss, then report it.
        Err(wikimedia::Miss::Missing) => {
            let _ = db::music::upsert_artist_image(state.db(), family.user_id, artist, None).await;
            return Err(AuthError::ItemNotFound);
        }
        // Nothing was learned about this artist, so nothing is remembered: the
        // next render asks again instead of showing a placeholder forever.
        Err(wikimedia::Miss::Unavailable) => return Err(AuthError::ItemNotFound),
    };

    // The credit is painted into the picture rather than carried beside it.
    // A Subsonic client is free to render the image and nothing else, so an
    // attribution held in a separate field is one that may never be shown —
    // and most Commons licences require it wherever the work appears.
    //
    // Both failures below are treated as "no image", not as "use it anyway":
    // an uncredited Commons photo is precisely what we may not serve. The miss
    // is cached like any other, so this does not re-fetch on every render.
    let Some(credit) = crate::metadata::watermark::credit_line(
        image.attribution.author.as_deref(),
        image.attribution.license.as_deref(),
    ) else {
        let _ = db::music::upsert_artist_image(state.db(), family.user_id, artist, None).await;
        return Err(AuthError::ItemNotFound);
    };

    match crate::metadata::watermark::burn(&image.bytes, &credit) {
        Ok(bytes) => {
            image.bytes = bytes;
            // Watermarking re-encodes, whatever came in.
            image.extension = "jpg";
            image.content_type = "image/jpeg";
        }
        Err(error) => {
            tracing::warn!(%artist, %error, "could not watermark artist image; dropping it");
            let _ = db::music::upsert_artist_image(state.db(), family.user_id, artist, None).await;
            return Err(AuthError::ItemNotFound);
        }
    }

    let content_type = image.content_type.to_string();
    let bytes = image.bytes.clone();
    store_artist_image(&state, &family, artist, image).await;
    Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response())
}

/// Best-effort, like every other cover write in this module: the caller already
/// has the bytes to answer with, so a storage failure costs a re-fetch next
/// time rather than the request.
async fn store_artist_image(
    state: &AppState,
    family: &FamilyContext,
    artist: &str,
    image: crate::metadata::wikimedia::ArtistImage,
) {
    let Ok(temp_file) = NamedTempFile::new() else { return };
    if tokio::fs::write(temp_file.path(), &image.bytes).await.is_err() {
        return;
    }
    let upload = TempUpload {
        file_name: format!("artist.{}", image.extension),
        temp_file,
        content_type: image.content_type.to_string(),
        size_bytes: image.bytes.len() as i64,
    };
    // Hashed, not the raw name: an artist name is free text and would otherwise
    // have to be made safe for an object key, where "AC/DC" alone would create
    // a spurious path segment.
    let key = crate::storage::family_key(
        family.family_id,
        format!(
            "music/{}/artists/{:x}.{}",
            family.user_id,
            <sha2::Sha256 as sha2::Digest>::digest(artist.to_lowercase().as_bytes()),
            image.extension
        ),
    );

    let Ok(mut tx) = state.db().begin().await else { return };
    let Ok(media_id) = store_temp_upload(state.storage(), &mut tx, &key, &upload).await else {
        return;
    };
    if db::music::upsert_fetched_artist_image(
        &mut *tx, family.user_id, artist, Some(media_id), &image.attribution,
    )
    .await
    .is_err()
    {
        return;
    }
    let _ = tx.commit().await;
}

/// GET /api/v1/music/artists/image-info?artist=… — who to credit for the
/// picture, and under what licence.
///
/// A separate call rather than headers on the image itself, because the client
/// renders the image with a plain image view that never sees response headers —
/// and because the attribution is text that belongs in the layout, not metadata
/// nobody reads. Most Commons licences *require* this to be shown, so it is not
/// decoration.
#[utoipa::path(get, path = "/artists/image-info", tag = "music", security(("bearer" = [])),
    params(ArtistImageQuery),
    responses((status = 200, body = ArtistImageInfoResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_artist_image_info(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<ArtistImageQuery>,
) -> Result<Json<ArtistImageInfoResponse>, AuthError> {
    let artist = params.artist.trim();
    if artist.is_empty() {
        return Err(AuthError::BadRequest("artist is required".to_string()));
    }

    let row = db::music::find_artist_image_attribution(state.db(), family.user_id, artist)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(ArtistImageInfoResponse {
        source: row.0,
        author: row.1,
        license: row.2,
        license_url: row.3,
        source_url: row.4,
        is_user_set: row.5,
    }))
}

/// POST /api/v1/music/artists/upload-image?artist=… — the user's own picture
/// for an artist, mirroring `upload_cover` for a track.
///
/// Marked `is_user_set`, which is what makes it permanent: the automatic
/// catalogue lookup skips any row carrying that flag, so a deliberate choice is
/// never quietly replaced by whatever a search returns later.
#[utoipa::path(post, path = "/artists/upload-image", tag = "music", security(("bearer" = [])),
    params(ArtistImageQuery),
    request_body(content_type = "multipart/form-data", description = "fields: `image` (the image file)"),
    responses((status = 201, description = "Stored"), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn upload_artist_image(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<ArtistImageQuery>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    let artist = params.artist.trim().to_string();
    if artist.is_empty() {
        return Err(AuthError::BadRequest("artist is required".to_string()));
    }

    let mut upload: Option<TempUpload> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        if field.name().unwrap_or_default() == "image" {
            upload = Some(save_field_to_temp(field).await?);
        }
    }
    let upload = upload.ok_or_else(|| AuthError::BadRequest("missing image payload".to_string()))?;

    let ext = file_extension(&upload.file_name).unwrap_or("jpg");
    let object_key = crate::storage::family_key(
        family.family_id,
        format!(
            "music/{}/artists/{:x}-user.{ext}",
            family.user_id,
            <sha2::Sha256 as sha2::Digest>::digest(artist.to_lowercase().as_bytes())
        ),
    );

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;
    db::music::set_user_artist_image(&mut *tx, family.user_id, &artist, media_id)
        .await
        .map_err(AuthError::Internal)?;
    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;

    Ok(StatusCode::CREATED)
}

/// DELETE /api/v1/music/artists/image?artist=… — forget this artist's picture,
/// user-set or fetched, and let the automatic lookup run again next time.
///
/// Deletes the row rather than nulling it: a NULL row is the cached "nothing
/// found", which would suppress the very lookup this is handing control back to.
#[utoipa::path(delete, path = "/artists/image", tag = "music", security(("bearer" = [])),
    params(ArtistImageQuery),
    responses((status = 204, description = "Forgotten"), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn delete_artist_image(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<ArtistImageQuery>,
) -> Result<StatusCode, AuthError> {
    let artist = params.artist.trim();
    if artist.is_empty() {
        return Err(AuthError::BadRequest("artist is required".to_string()));
    }
    db::music::clear_artist_image(state.db(), family.user_id, artist)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/music/albums?artist=…
/// Albums the caller can see, grouped by album artist.
#[utoipa::path(get, path = "/albums", tag = "music", security(("bearer" = [])),
    params(AlbumsQuery),
    responses((status = 200, body = Vec<AlbumSummary>)))]
async fn list_albums(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<AlbumsQuery>,
) -> Result<Json<Vec<AlbumSummary>>, AuthError> {
    let rows = db::music::list_albums(state.db(), family.viewer(), q.artist.as_deref())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(artist, album, track_count, duration_secs, cover_track)| AlbumSummary {
                artist,
                album,
                track_count,
                duration_secs,
                cover_url: cover_track
                    .map(|id| format!("/api/v1/music/tracks/{id}/cover")),
            })
            .collect(),
    ))
}

/// GET /api/v1/music/genres
/// Genres with their track counts.
#[utoipa::path(get, path = "/genres", tag = "music", security(("bearer" = [])),
    responses((status = 200, body = Vec<GenreSummary>)))]
async fn list_genres(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<GenreSummary>>, AuthError> {
    let rows = db::music::list_genres(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(genre, track_count)| GenreSummary { genre, track_count })
            .collect(),
    ))
}

const STREAM_EXPIRY_SECS: u64 = 4 * 3600;

// ── Track handlers ────────────────────────────────────────────────────────

/// The whole list, as before, but streamed: rows go out as PostgreSQL returns
/// them, so the server holds a few hundred tracks at a time whatever the
/// catalog's size (docs/CAPACITY.md, issue #2). Same JSON array as ever.
#[utoipa::path(get, path = "/tracks", tag = "music", security(("bearer" = [])),
    responses((status = 200, description = "Every track the caller can play, streamed as one array", body = Vec<TrackResponse>)))]
async fn list_tracks(family: FamilyContext, State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(db::music::send_tracks(state.db().clone(), family.viewer(), tx));
    let viewer_id = family.user_id;
    let body = crate::http::json_stream::array(rx, Vec::new(), b"", "track list", move |track| {
        track_to_response(track, viewer_id)
    });
    ([(axum::http::header::CONTENT_TYPE, "application/json")], body)
        .into_response()
}

/// GET /api/v1/music/duplicates — DEDUPLICATION_PLAN.md P3. Exact-duplicate (`sha256`-matched)
/// groups of the caller's own tracks only. Rows arrive pre-sorted by hash then upload date
/// (`db::music::duplicate_tracks`), so building groups is one linear pass, not a second query
/// per group.
#[utoipa::path(get, path = "/duplicates", tag = "music", security(("bearer" = [])),
    responses((status = 200, body = DuplicatesResponse)))]
async fn list_duplicates(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<DuplicatesResponse>, AuthError> {
    let rows = db::music::duplicate_tracks(state.db(), family.user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut groups: Vec<DuplicateGroupResponse> = Vec::new();
    for row in rows {
        let sha256 = row.sha256.clone();
        let track = duplicate_track_to_response(row);
        match groups.last_mut() {
            Some(g) if g.tracks.last().is_some_and(|t| t.sha256 == sha256) => g.tracks.push(track),
            _ => groups.push(DuplicateGroupResponse { tier: "identical".to_string(), tracks: vec![track] }),
        }
    }

    Ok(Json(DuplicatesResponse { groups }))
}

fn duplicate_track_to_response(row: db::music::DuplicateTrackRow) -> DuplicateTrackResponse {
    DuplicateTrackResponse {
        cover_url: row
            .cover_object_id
            .map(|_| format!("/api/v1/music/tracks/{}/cover", row.id)),
        visibility: access::visibility_of(row.family_id).to_string(),
        id: row.id.to_string(),
        title: row.title,
        artist: row.artist,
        album: row.album,
        genre: row.genre,
        track_number: row.track_number,
        duration_secs: row.duration_secs,
        musicbrainz_recording_id: row.musicbrainz_recording_id,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        sha256: row.sha256,
        size_bytes: row.size_bytes,
        content_type: row.content_type,
        playlist_count: row.playlist_count,
        is_starred: row.is_starred,
    }
}

/// GET /api/v1/music/tracks/{id} — one track the caller can play.
#[utoipa::path(get, path = "/tracks/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, body = TrackResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_track(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TrackResponse>, AuthError> {
    let track = db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    Ok(Json(track_to_response(track, family.user_id)))
}

/// PUT /api/v1/music/tracks/:id/visibility — move a track between the
/// private and family folders. Owner only.
#[utoipa::path(put, path = "/tracks/{id}/visibility", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = SetVisibilityRequest,
    responses((status = 200, body = TrackResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn set_track_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetVisibilityRequest>,
) -> Result<Json<TrackResponse>, AuthError> {
    let shared = parse_visibility(&body.visibility)?;

    let updated = access::set_shared(
        state.db(),
        family.user_id,
        family.family_id,
        access::MUSIC,
        id,
        shared,
    )
    .await
    .map_err(AuthError::Internal)?;

    if !updated {
        return Err(AuthError::ItemNotFound);
    }

    let fresh = db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(track_to_response(fresh, family.user_id)))
}

/// `private` | `family` → whether the item should carry a family_id.
fn parse_visibility(value: &str) -> Result<bool, AuthError> {
    match value.trim() {
        access::VIS_PRIVATE => Ok(false),
        access::VIS_FAMILY => Ok(true),
        _ => Err(AuthError::BadRequest(
            "visibility must be 'private' or 'family'".to_string(),
        )),
    }
}

/// POST /api/v1/music/tracks/upload — multipart upload of a single track
#[utoipa::path(post, path = "/tracks/upload", tag = "music", security(("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "fields: `file` (the audio, required), `cover` (image), `title`, `artist`, `album`, `album_artist`, `genre`, `track_number`, `duration_secs`, `visibility` (`private` or `family`); tags the form leaves out are read from the file"),
    responses((status = 201, body = TrackResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 403, description = "Not allowed to upload", body = crate::http::openapi::ErrorBody)))]
async fn upload_track(
    family: FamilyContext,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<TrackResponse>), AuthError> {
    family.require_can_upload()?;
    let mut title: Option<String> = None;
    let mut artist: Option<String> = None;
    let mut album: Option<String> = None;
    let mut album_artist: Option<String> = None;
    let mut genre: Option<String> = None;
    let mut track_number: Option<i32> = None;
    let mut duration_secs: Option<i32> = None;
    let mut visibility: Option<String> = None;
    let mut audio_upload: Option<TempUpload> = None;
    let mut cover_upload: Option<TempUpload> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "title" => title = Some(read_text_field(field).await?),
            "artist" => artist = Some(read_text_field(field).await?),
            "album" => album = Some(read_text_field(field).await?),
            "album_artist" => album_artist = Some(read_text_field(field).await?),
            "genre" => genre = Some(read_text_field(field).await?),
            "track_number" => {
                let raw = read_text_field(field).await?;
                track_number = Some(raw.parse::<i32>()
                    .map_err(|e| AuthError::BadRequest(format!("invalid track_number: {e}")))?);
            }
            "duration_secs" => {
                let raw = read_text_field(field).await?;
                duration_secs = Some(raw.parse::<i32>()
                    .map_err(|e| AuthError::BadRequest(format!("invalid duration_secs: {e}")))?);
            }
            "visibility" => visibility = Some(read_text_field(field).await?),
            "file" => audio_upload = Some(save_field_to_temp(field).await?),
            "cover" => cover_upload = Some(save_field_to_temp(field).await?),
            _ => { let _ = field; }
        }
    }

    // Which folder the upload lands in. Absent ⇒ private (fail closed).
    let family_id =
        access::family_id_for_visibility(visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let audio = audio_upload
        .ok_or_else(|| AuthError::BadRequest("missing audio file".to_string()))?;

    let object_key_audio = crate::storage::family_key(
        family.family_id,
        format!("music/{}/{}", family.user_id, sanitize(&audio.file_name)),
    );
    let fields = TrackFields { title, artist, album, album_artist, genre, track_number, duration_secs };
    let track = create_track(
        &state,
        &family,
        family_id,
        fields,
        &audio.file_name,
        audio.temp_file.path(),
        AudioSource::Temp { object_key: &object_key_audio, upload: &audio },
        cover_upload.as_ref(),
    )
    .await?;
    let _ = crate::filesync::paths::ensure_track(state.db(), track.id)
        .await
        .inspect_err(|e| tracing::warn!(track_id = %track.id, "no path for the new track: {e:#}"));

    Ok((StatusCode::CREATED, Json(track_to_response(track, family.user_id))))
}

/// Body of `POST /music/tracks/from-upload` — the direct-to-storage twin of
/// the multipart upload, without its 100 MB Cloudflare limit.
#[derive(Deserialize, ToSchema)]
pub struct FromUploadRequest {
    /// Key returned by `/uploads/presign` (kind `music_track`), already PUT.
    pub object_key: String,
    /// Where the file sits in the own.audio folder (`Music/…`), for a file
    /// put there. Without one the server gives it a default path.
    #[serde(default)]
    pub path: Option<String>,
    /// The file's own name, for a title when the tags have none.
    pub original_filename: String,
    /// `private` (default) or `family`.
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub track_number: Option<i32>,
    #[serde(default)]
    pub duration_secs: Option<i32>,
}

/// A created track and where it sits in the own.audio folder — the path
/// asked for, or with ` (2)` when another of the owner's files holds it.
#[derive(Serialize, ToSchema)]
pub struct TrackWithPath {
    #[serde(flatten)]
    pub track: TrackResponse,
    pub path: Option<String>,
}

/// POST /api/v1/music/tracks/from-upload
/// Create a track from a file already uploaded to storage.
#[utoipa::path(post, path = "/tracks/from-upload", tag = "music", security(("bearer" = [])),
    request_body = FromUploadRequest,
    responses((status = 201, body = TrackWithPath), (status = 400, body = crate::http::openapi::ErrorBody), (status = 401, description = "The key is not in the caller's family storage", body = crate::http::openapi::ErrorBody), (status = 403, description = "Not allowed to upload", body = crate::http::openapi::ErrorBody)))]
async fn create_track_from_upload(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<FromUploadRequest>,
) -> Result<(StatusCode, Json<TrackWithPath>), AuthError> {
    family.require_can_upload()?;
    crate::uploads::require_own_key(&family, &body.object_key)?;
    let path = body
        .path
        .as_deref()
        .map(|p| crate::filesync::paths::normalize(p, crate::filesync::paths::Kind::MusicTrack))
        .transpose()
        .map_err(AuthError::BadRequest)?;
    let family_id = access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
        .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let (size_bytes, content_type) = state
        .storage()
        .head_object(&body.object_key)
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("no object was uploaded under that key".to_string()))?;
    // The tags are read from the stored file itself, exactly as the multipart
    // route reads them from its temp file.
    let local = state.storage().download_to_temp(&body.object_key).await.map_err(AuthError::Internal)?;

    let fields = TrackFields {
        title: body.title,
        artist: body.artist,
        album: body.album,
        album_artist: body.album_artist,
        genre: body.genre,
        track_number: body.track_number,
        duration_secs: body.duration_secs,
    };
    let track = create_track(
        &state,
        &family,
        family_id,
        fields,
        &body.original_filename,
        local.path(),
        AudioSource::Stored { object_key: &body.object_key, content_type: &content_type, size_bytes },
        None,
    )
    .await?;

    let assigned = match path {
        Some(wanted) => crate::filesync::paths::claim(
            state.db(),
            family.user_id,
            crate::filesync::paths::Kind::MusicTrack,
            track.id,
            &wanted,
        )
        .await
        .map(Some),
        None => crate::filesync::paths::ensure_track(state.db(), track.id).await,
    }
    .map_err(AuthError::Internal)?;
    // An image already next to it can be its cover (file-sync-plan §2 item 16).
    if let Some(path) = &assigned {
        let dir = crate::filesync::paths::parent_dir(path);
        // A file without a disc tag, in a folder named like `CD 2`, is on that disc.
        let from_folder = crate::filesync::paths::disc_from_folder(dir);
        if let (None, Some(disc)) = (track.disc_number, from_folder) {
            db::music::set_disc(state.db(), track.id, Some(disc), None)
                .await
                .map_err(AuthError::Internal)?;
        }
        crate::filesync::companion::apply_default_cover(state.db(), family.user_id, dir)
            .await
            .map_err(AuthError::Internal)?;
    }
    let track = if assigned.is_some() {
        db::music::find_track_owned(state.db(), track.id, family.user_id)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::ItemNotFound)?
    } else {
        track
    };

    Ok((
        StatusCode::CREATED,
        Json(TrackWithPath { track: track_to_response(track, family.user_id), path: assigned }),
    ))
}

/// What a client may say about a track; the rest comes from the file's tags.
struct TrackFields {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    genre: Option<String>,
    track_number: Option<i32>,
    duration_secs: Option<i32>,
}

/// Where a new track's audio comes from.
enum AudioSource<'a> {
    /// Streamed through this server; pushed to storage inside the transaction.
    Temp { object_key: &'a str, upload: &'a TempUpload },
    /// Already in storage (`/uploads/presign`); only registered.
    Stored { object_key: &'a str, content_type: &'a str, size_bytes: i64 },
}

/// Create a track around its audio. Whatever the client did not send is read
/// from the file's own tags at `local_audio` — most clients (confirmed: the
/// Mac app) never read tags before uploading, so without this a plain upload
/// lands with nothing but a filename-derived title. See `read_embedded_tags`.
/// An explicit cover wins over an embedded one. Queues the checksum.
#[allow(clippy::too_many_arguments)]
async fn create_track(
    state: &AppState,
    family: &FamilyContext,
    family_id: Option<Uuid>,
    fields: TrackFields,
    file_name: &str,
    local_audio: &StdPath,
    audio: AudioSource<'_>,
    cover_upload: Option<&TempUpload>,
) -> Result<models::MusicTrack, AuthError> {
    let TrackFields { title, mut artist, mut album, mut album_artist, mut genre, mut track_number, duration_secs } = fields;
    let embedded = read_embedded_tags(local_audio);
    if artist.is_none() { artist = embedded.artist.clone(); }
    if album.is_none() { album = embedded.album.clone(); }
    // A compilation without an album-artist tag is still one album, not one per artist.
    if album_artist.is_none() {
        album_artist = embedded
            .album_artist
            .clone()
            .or_else(|| embedded.is_compilation.then(|| VARIOUS_ARTISTS.to_string()));
    }
    if genre.is_none() { genre = embedded.genre.clone(); }
    if track_number.is_none() { track_number = embedded.track_number; }

    let normalized_title = title
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| embedded.title.clone())
        .unwrap_or_else(|| infer_title(file_name));

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let audio_media_id = match audio {
        AudioSource::Temp { object_key, upload } => store_temp_upload(state.storage(), &mut tx, object_key, upload).await?,
        AudioSource::Stored { object_key, content_type, size_bytes } => {
            db::media::upsert_object(&mut *tx, state.storage().bucket(), object_key, content_type, Some(size_bytes))
                .await
                .map_err(AuthError::Internal)?
        }
    };

    let track = db::music::insert_track(
        &mut *tx,
        family.user_id,
        family_id,
        &normalized_title,
        trim_opt(artist.as_deref()),
        trim_opt(album.as_deref()),
        trim_opt(album_artist.as_deref()),
        embedded.is_compilation,
        trim_opt(genre.as_deref()),
        track_number,
        duration_secs,
        audio_media_id,
    )
    .await
    .map_err(AuthError::Internal)?;
    db::music::set_disc(&mut *tx, track.id, embedded.disc_number, embedded.disc_total)
        .await
        .map_err(AuthError::Internal)?;
    let mut track = track;
    track.disc_number = embedded.disc_number.filter(|n| *n > 0);
    track.disc_total = embedded.disc_total.filter(|n| *n > 0);

    if let Some(cover) = cover_upload {
        let ext = file_extension(&cover.file_name).unwrap_or("jpg");
        let cover_key = crate::storage::family_key(
            family.family_id,
            format!("music/{}/covers/{}.{ext}", family.user_id, track.id),
        );
        let cover_media_id = store_temp_upload(state.storage(), &mut tx, &cover_key, cover).await?;
        db::music::update_track_cover(&mut *tx, track.id, family.user_id, cover_media_id)
            .await
            .map_err(AuthError::Internal)?;
    } else if let Some((bytes, ext, content_type)) = embedded.cover {
        let cover_key = crate::storage::family_key(
            family.family_id,
            format!("music/{}/covers/{}.{ext}", family.user_id, track.id),
        );
        let cover = write_temp_upload(&bytes, format!("cover.{ext}"), content_type.to_string())?;
        let cover_media_id = store_temp_upload(state.storage(), &mut tx, &cover_key, &cover).await?;
        db::music::update_track_cover(&mut *tx, track.id, family.user_id, cover_media_id)
            .await
            .map_err(AuthError::Internal)?;
    }

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;

    // DEDUPLICATION_PLAN.md P2 — best-effort: a failed enqueue shouldn't fail an otherwise-
    // successful upload. The backfill sweep (`jobs::worker`) will pick this object up later
    // regardless if this particular enqueue is ever lost.
    let checksum_payload = serde_json::json!({ "media_object_id": audio_media_id.to_string() });
    if let Err(e) = db::jobs::enqueue(state.db(), "media_checksum", Some(&checksum_payload), None).await {
        tracing::warn!(track_id = %track.id, "failed to enqueue media checksum job: {e}");
    }

    db::music::find_track_owned(state.db(), track.id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)
}

/// A file in a read-only library folder becomes a track (`crate::library_folders`).
/// Tags are read from the file in place; `sidecar_cover` (a `cover.jpg` next
/// to it, already a folder key) is used when the file has no picture of its
/// own. Returns the new track's id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_folder_track(
    state: &AppState,
    family: &FamilyContext,
    family_id: Option<Uuid>,
    file: &StdPath,
    object_key: &str,
    content_type: &str,
    size_bytes: i64,
    duration_secs: Option<i32>,
    sidecar_cover: Option<(&str, &str, i64)>,
) -> anyhow::Result<Uuid> {
    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("track");
    let fields = TrackFields {
        title: None,
        artist: None,
        album: None,
        album_artist: None,
        genre: None,
        track_number: None,
        duration_secs,
    };
    let track = create_track(
        state,
        family,
        family_id,
        fields,
        file_name,
        file,
        AudioSource::Stored { object_key, content_type, size_bytes },
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("create track: {e}"))?;
    if track.cover_object_id.is_none() {
        if let Some((key, cover_type, cover_size)) = sidecar_cover {
            let media = db::media::upsert_object(state.db(), state.storage().bucket(), key, cover_type, Some(cover_size)).await?;
            db::music::update_track_cover(state.db(), track.id, family.user_id, media).await?;
        }
    }
    sqlx::query("UPDATE music_tracks_all SET source = 'folder' WHERE id = $1")
        .bind(track.id)
        .execute(state.db())
        .await?;
    crate::filesync::paths::ensure_track(state.db(), track.id).await?;
    Ok(track.id)
}

/// A folder file changed on disk: read its tags again, in place. Like the
/// manual rescan, the file wins where it has a value and the stored value
/// survives where it has none; a track identified through MusicBrainz keeps
/// its identification.
pub(crate) async fn refresh_folder_track(
    state: &AppState,
    owner_id: Uuid,
    track_id: Uuid,
    file: &StdPath,
    size_bytes: i64,
    duration_secs: Option<i32>,
) -> anyhow::Result<()> {
    let Some(track) = db::music::find_track_owned(state.db(), track_id, owner_id).await? else {
        return Ok(()); // removed by someone: stays removed
    };
    sqlx::query("UPDATE media_objects SET size_bytes = $2 WHERE id = $1")
        .bind(track.audio_object_id)
        .bind(size_bytes)
        .execute(state.db())
        .await?;
    if let Some(d) = duration_secs {
        sqlx::query("UPDATE music_tracks_all SET duration_secs = $2 WHERE id = $1")
            .bind(track_id)
            .bind(d)
            .execute(state.db())
            .await?;
    }
    if track.musicbrainz_recording_id.is_some() {
        return Ok(());
    }
    let path = file.to_path_buf();
    let tags = tokio::task::spawn_blocking(move || read_embedded_tags(&path)).await?;
    db::music::update_track(
        state.db(),
        track_id,
        owner_id,
        tags.title.as_deref().unwrap_or(&track.title),
        tags.artist.as_deref().or(track.artist.as_deref()),
        tags.album.as_deref().or(track.album.as_deref()),
        tags.album_artist
            .as_deref()
            .or(tags.is_compilation.then_some(VARIOUS_ARTISTS))
            .map(Some),
        tags.genre.as_deref().or(track.genre.as_deref()),
        tags.track_number.or(track.track_number),
    )
    .await?;
    db::music::set_disc(state.db(), track_id, tags.disc_number, tags.disc_total).await?;
    Ok(())
}

/// PUT /api/v1/music/tracks/{id} — edit a track's metadata. Owner only.
#[utoipa::path(put, path = "/tracks/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = UpdateTrackRequest,
    responses((status = 200, body = TrackResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn update_track(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTrackRequest>,
) -> Result<Json<TrackResponse>, AuthError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AuthError::BadRequest("title is required".to_string()));
    }

    // Editing is owner-only: a family member who can play a shared track
    // must not be able to rewrite its metadata.
    db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::music::update_track(
        state.db(),
        id,
        family.user_id,
        title,
        trim_opt(body.artist.as_deref()),
        trim_opt(body.album.as_deref()),
        body.album_artist.as_ref().map(|v| trim_opt(v.as_deref())),
        trim_opt(body.genre.as_deref()),
        body.track_number,
    )
    .await
    .map_err(AuthError::Internal)?;

    let fresh = db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(track_to_response(fresh, family.user_id)))
}

/// GET /api/v1/music/tracks/:id/metadata/file-tags
///
/// What the audio file itself says, read fresh from storage and **not**
/// written anywhere. The read-only twin of `rescan`, and deliberately without
/// its "refuses once identified" guard: the moment you most need to see the
/// file's own tags is right after a MusicBrainz match got it wrong, which is
/// exactly when `rescan` won't speak to you.
///
/// The two sources drift by design — applying a MusicBrainz recording updates
/// the database and never rewrites the file — so a track can sit in the
/// library as one album while its bytes still say another.
#[utoipa::path(get, path = "/tracks/{id}/metadata/file-tags", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, body = FileTagsResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_track_file_tags(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<FileTagsResponse>, AuthError> {
    let track = db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let key: String = sqlx::query_scalar("SELECT object_key FROM media_objects WHERE id = $1")
        .bind(track.audio_object_id)
        .fetch_one(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    // Streamed to disk: a FLAC is easily 100–300 MB.
    let temp = state.storage().download_to_temp(&key).await.map_err(AuthError::Internal)?;
    let tags = read_embedded_tags(temp.path());

    Ok(Json(FileTagsResponse {
        file_name: key.rsplit('/').next().map(ToOwned::to_owned),
        title: tags.title,
        artist: tags.artist,
        album: tags.album,
        album_artist: tags.album_artist,
        is_compilation: tags.is_compilation,
        genre: tags.genre,
        track_number: tags.track_number,
        has_cover: tags.cover.is_some(),
    }))
}

#[derive(Serialize, ToSchema)]
struct DiscsResponse {
    checked: usize,
    left: i64,
}

/// POST /api/v1/music/tracks/discs — reads the disc number of up to 20 of the caller's
/// tracks uploaded before it was kept, and says how many are left; a client calls again until
/// none are. Only the disc fields change, so an identified track is safe too. A file that
/// cannot be read counts as read: it has nothing to add.
#[utoipa::path(post, path = "/tracks/discs", tag = "music", security(("bearer" = [])),
    responses((status = 200, body = DiscsResponse)))]
async fn read_discs(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<DiscsResponse>, AuthError> {
    let (batch, _) = db::music::tracks_without_disc(state.db(), family.user_id, 20)
        .await
        .map_err(AuthError::Internal)?;
    for (id, key) in &batch {
        let tags = match state.storage().download_to_temp(key).await {
            Ok(temp) => read_embedded_tags(temp.path()),
            Err(e) => {
                tracing::warn!(track = %id, "disc number unreadable: {e:#}");
                EmbeddedTags::default()
            }
        };
        db::music::set_disc(state.db(), *id, tags.disc_number, tags.disc_total)
            .await
            .map_err(AuthError::Internal)?;
    }
    let (_, left) = db::music::tracks_without_disc(state.db(), family.user_id, 0)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(DiscsResponse { checked: batch.len(), left }))
}

/// POST /api/v1/music/tracks/:id/metadata/rescan — owner-only. Re-reads the
/// track's own embedded tags from the file already in storage.
///
/// Exists because `read_embedded_tags` was added to `upload_track` *after*
/// libraries had already been uploaded. Those tracks kept a filename-derived
/// title and nothing else — confirmed on a real library: 199 tracks with no
/// artist, no album, no genre, whose files carried all four the whole time.
/// Re-uploading them to recover data already sitting in storage would be
/// absurd; this reads it in place.
///
/// Distinct from the MusicBrainz flow, and not a replacement for it: this
/// trusts the file and adds no MBIDs. It refuses on a track that has already
/// been identified, because MusicBrainz is the better source and silently
/// overwriting a deliberate match with whatever a tagger once wrote would be a
/// regression the user never asked for.
#[utoipa::path(post, path = "/tracks/{id}/metadata/rescan", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, body = TrackResponse), (status = 400, description = "Already identified, or the file has no tags", body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn rescan_track_tags(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TrackResponse>, AuthError> {
    let track = db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if track.musicbrainz_recording_id.is_some() {
        return Err(AuthError::BadRequest(
            "this track was identified via MusicBrainz; re-reading its file tags would overwrite that".to_string(),
        ));
    }

    let key: String = sqlx::query_scalar("SELECT object_key FROM media_objects WHERE id = $1")
        .bind(track.audio_object_id)
        .fetch_one(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    // `lofty` reads from a path; the object is streamed into a temp file that
    // goes away when this returns, never held in memory whole.
    let temp = state
        .storage()
        .download_to_temp(&key)
        .await
        .map_err(AuthError::Internal)?;
    let tags = read_embedded_tags(temp.path());

    if tags.title.is_none()
        && tags.artist.is_none()
        && tags.album.is_none()
        && tags.genre.is_none()
        && tags.track_number.is_none()
    {
        return Err(AuthError::BadRequest(
            "this file carries no embedded tags to read".to_string(),
        ));
    }

    // The file wins where it has something, and the existing value survives
    // where it doesn't — so a rescan can only ever add information. The title
    // is the exception worth noting: it is never null, so without preferring
    // the tag the filename-derived title this exists to replace would stay.
    db::music::update_track(
        state.db(),
        id,
        family.user_id,
        tags.title.as_deref().unwrap_or(&track.title),
        tags.artist.as_deref().or(track.artist.as_deref()),
        tags.album.as_deref().or(track.album.as_deref()),
        // Only a tag the file actually has may replace the stored value, same as every other
        // field here.
        tags.album_artist
            .as_deref()
            .or(tags.is_compilation.then_some(VARIOUS_ARTISTS))
            .map(Some),
        tags.genre.as_deref().or(track.genre.as_deref()),
        tags.track_number.or(track.track_number),
    )
    .await
    .map_err(AuthError::Internal)?;

    db::music::set_disc(state.db(), id, tags.disc_number, tags.disc_total)
        .await
        .map_err(AuthError::Internal)?;

    if let Some((cover_bytes, ext, content_type)) = tags.cover {
        store_rescanned_cover(&state, &family, id, cover_bytes, ext, content_type).await;
    }

    let fresh = db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(track_to_response(fresh, family.user_id)))
}

/// Best-effort, exactly like `apply_cover_art`: the metadata update has already
/// succeeded by the time this runs, so a failed cover must not fail the rescan.
async fn store_rescanned_cover(
    state: &AppState,
    family: &FamilyContext,
    track_id: Uuid,
    bytes: Vec<u8>,
    ext: &str,
    content_type: &str,
) {
    let Ok(temp_file) = NamedTempFile::new() else { return };
    if tokio::fs::write(temp_file.path(), &bytes).await.is_err() {
        return;
    }
    let upload = TempUpload {
        file_name: format!("cover.{ext}"),
        temp_file,
        content_type: content_type.to_string(),
        size_bytes: bytes.len() as i64,
    };
    let cover_key = crate::storage::family_key(
        family.family_id,
        format!("music/{}/covers/{}.{ext}", family.user_id, track_id),
    );
    let Ok(mut tx) = state.db().begin().await else { return };
    let Ok(cover_media_id) = store_temp_upload(state.storage(), &mut tx, &cover_key, &upload).await
    else {
        return;
    };
    if db::music::update_track_cover(&mut *tx, track_id, family.user_id, cover_media_id)
        .await
        .is_err()
    {
        return;
    }
    let _ = tx.commit().await;
}

/// The metadata mirror, or a clear error when it isn't configured.
///
/// Deliberately an error rather than a fallback to the public MusicBrainz API:
/// that path is rate-limited to ~1 req/s for the entire deployment, so silently
/// dropping onto it would turn a misconfiguration into an unexplained slowdown
/// exactly when load is highest. See docs/music-metadata-plan.md.
fn metadata_mirror(state: &AppState) -> Result<crate::metadata::mirror::MetadataMirror<'_>, AuthError> {
    state
        .config()
        .metadata
        .as_ref()
        .map(crate::metadata::mirror::MetadataMirror::new)
        .ok_or_else(|| {
            AuthError::Internal(anyhow::anyhow!(
                "music metadata service is not configured (METADATA__BASE_URL / METADATA__API_KEY)"
            ))
        })
}

/// POST /api/v1/music/tracks/:id/metadata/search — MusicBrainz recording
/// candidates for this track. The track id only scopes visibility/auth; the
/// search itself runs on whatever title/artist/album the client sends
/// (typically the track's current values, editable before searching).
#[utoipa::path(post, path = "/tracks/{id}/metadata/search", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = MetadataSearchRequest,
    responses((status = 200, body = Vec<crate::metadata::MetadataCandidate>), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn search_track_metadata(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<MetadataSearchRequest>,
) -> Result<Json<Vec<crate::metadata::MetadataCandidate>>, AuthError> {
    let track = db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if body.title.is_none() && body.artist.is_none() && body.album.is_none() {
        return Err(AuthError::BadRequest(
            "at least one of title, artist, album is required".to_string(),
        ));
    }

    // The stored duration is free, strong evidence the public API could never
    // use: MusicBrainz holds many live and compilation versions of a popular
    // track under one name, and length is what separates them.
    let duration_ms = track
        .duration_secs
        .filter(|secs| *secs > 0)
        .map(|secs| secs as u64 * 1000);

    let results = metadata_mirror(&state)?
        .identify(
            body.title.as_deref(),
            body.artist.as_deref(),
            body.album.as_deref(),
            duration_ms,
            body.limit,
        )
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(results))
}

#[derive(Deserialize, ToSchema)]
pub struct AlbumIdentifyRequest {
    pub track_ids: Vec<Uuid>,
}

/// An album candidate, with each match pointing at one of the request's
/// tracks rather than at its position in the list.
#[derive(Serialize, ToSchema)]
pub struct AlbumCandidateResponse {
    pub mb_release_id: String,
    pub mb_release_group_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub status: Option<String>,
    pub track_count: i32,
    pub matched: u32,
    pub editions: u32,
    pub cover_art_url: Option<String>,
    pub tracks: Vec<AlbumTrackMatchResponse>,
}

#[derive(Serialize, ToSchema)]
pub struct AlbumTrackMatchResponse {
    pub track_id: Uuid,
    pub mb_recording_id: String,
    pub title: String,
    pub disc: i32,
    pub position: i32,
}

/// The value most of the tracks share, ignoring blanks — what a group's
/// artist or album tag most likely is when a few files disagree.
fn most_common<'a>(values: impl Iterator<Item = Option<&'a str>>) -> Option<&'a str> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for v in values.flatten().map(str::trim).filter(|v| !v.is_empty()) {
        match counts.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(v)) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v)
}

/// POST /api/v1/music/albums/identify — which album a group of tracks is.
///
/// Matching each track on its own let an album's songs scatter over live
/// bootlegs and compilations, because each recording resolved to a release
/// without knowing about the others. This asks once for the whole group; the
/// client then applies the chosen album's matches with the existing per-track
/// apply, so nothing about how a match is stored changes.
#[utoipa::path(post, path = "/albums/identify", tag = "music", security(("bearer" = [])),
    request_body = AlbumIdentifyRequest,
    responses((status = 200, body = Vec<AlbumCandidateResponse>), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn identify_album(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<AlbumIdentifyRequest>,
) -> Result<Json<Vec<AlbumCandidateResponse>>, AuthError> {
    if body.track_ids.is_empty() || body.track_ids.len() > 60 {
        return Err(AuthError::BadRequest("between 1 and 60 tracks".to_string()));
    }
    let mut tracks = Vec::with_capacity(body.track_ids.len());
    for id in &body.track_ids {
        let track = db::music::find_track(state.db(), *id, family.viewer())
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::ItemNotFound)?;
        tracks.push(track);
    }

    // No artist, no album search: a bare title is on thousands of releases.
    // Empty is the honest answer, and the client falls back to per-track.
    let Some(artist) = most_common(tracks.iter().map(|t| t.artist.as_deref())) else {
        return Ok(Json(Vec::new()));
    };
    let album = most_common(tracks.iter().map(|t| t.album.as_deref()));
    let queries = tracks
        .iter()
        .map(|t| crate::metadata::mirror::AlbumTrackQuery {
            title: t.title.as_str(),
            duration_ms: t.duration_secs.filter(|s| *s > 0).map(|s| s as u64 * 1000),
        })
        .collect();

    let candidates = metadata_mirror(&state)?
        .identify_album(artist, album, queries)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        candidates
            .into_iter()
            .map(|c| AlbumCandidateResponse {
                tracks: c
                    .tracks
                    .into_iter()
                    .filter_map(|m| {
                        body.track_ids.get(m.index as usize).map(|&track_id| AlbumTrackMatchResponse {
                            track_id,
                            mb_recording_id: m.mb_recording_id,
                            title: m.title,
                            disc: m.disc,
                            position: m.position,
                        })
                    })
                    .collect(),
                mb_release_id: c.mb_release_id,
                mb_release_group_id: c.mb_release_group_id,
                title: c.title,
                artist: c.artist,
                year: c.year,
                primary_type: c.primary_type,
                secondary_types: c.secondary_types,
                status: c.status,
                track_count: c.track_count,
                matched: c.matched,
                editions: c.editions,
                cover_art_url: c.cover_art_url,
            })
            .collect(),
    ))
}

/// POST /api/v1/music/tracks/:id/metadata/apply — owner-only, mirrors
/// `update_track`'s auth. Re-fetches the chosen recording from MusicBrainz
/// by id rather than trusting a client-supplied copy of a search result.
#[utoipa::path(post, path = "/tracks/{id}/metadata/apply", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = ApplyMetadataRequest,
    responses((status = 200, body = TrackResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn apply_track_metadata(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyMetadataRequest>,
) -> Result<Json<TrackResponse>, AuthError> {
    db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let recording = metadata_mirror(&state)?
        .recording(
            &body.mb_recording_id,
            body.mb_release_id.as_deref(),
        )
        .await
        .map_err(AuthError::Internal)?;

    db::music::apply_musicbrainz_metadata(
        state.db(),
        id,
        family.user_id,
        &recording.title,
        recording.artist.as_deref(),
        recording.album.as_deref(),
        recording.genre.as_deref(),
        recording.track_number,
        &recording.mb_recording_id,
        recording.mb_release_id.as_deref(),
        recording.mb_artist_id.as_deref(),
        recording.album_artist.as_deref(),
        recording.mb_release_group_id.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    if body.fetch_cover {
        // No longer gated on a release id: the cascade's search-based providers
        // match on artist + album, so a recording MusicBrainz could identify but
        // not place on a release can still get a cover.
        apply_cover_art(&state, &family, id, &recording).await;
    }

    let fresh = db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(track_to_response(fresh, family.user_id)))
}

/// Best-effort: a missing/unreachable cover never fails the apply, since
/// the metadata update itself already succeeded by the time this runs.
async fn apply_cover_art(
    state: &AppState,
    family: &FamilyContext,
    track_id: Uuid,
    recording: &crate::metadata::RecordingDetail,
) {
    let query = crate::metadata::cover_art::CoverQuery {
        mb_release_id: recording.mb_release_id.as_deref(),
        artist: recording.artist.as_deref(),
        album: recording.album.as_deref(),
    };

    // The cascade has already downloaded and sniffed the bytes — a provider
    // whose response was unreachable, or was an error page wearing an
    // `image/jpeg` content type, was skipped in favour of the next one rather
    // than ending the attempt.
    let Some(cover) = crate::metadata::cover_art::resolve(&query).await else {
        tracing::debug!(
            track_id = %track_id,
            "no cover art source had a usable image; leaving the track's cover unchanged"
        );
        return;
    };

    tracing::info!(track_id = %track_id, source = cover.source, "cover art resolved");
    let bytes = cover.bytes;
    let format = cover.format;
    let ext = format.extension();
    let content_type = format.content_type();

    let Ok(temp_file) = NamedTempFile::new() else {
        return;
    };
    if tokio::fs::write(temp_file.path(), &bytes).await.is_err() {
        return;
    }
    let upload = TempUpload {
        file_name: format!("cover.{ext}"),
        temp_file,
        content_type: content_type.to_string(),
        size_bytes: bytes.len() as i64,
    };

    let cover_key = crate::storage::family_key(
        family.family_id,
        format!("music/{}/covers/{}.{ext}", family.user_id, track_id),
    );

    let Ok(mut tx) = state.db().begin().await else {
        return;
    };
    let Ok(cover_media_id) = store_temp_upload(state.storage(), &mut tx, &cover_key, &upload).await
    else {
        return;
    };
    if db::music::update_track_cover(&mut *tx, track_id, family.user_id, cover_media_id)
        .await
        .is_err()
    {
        return;
    }
    let _ = tx.commit().await;
}

/// Moves the track to the trash (30 days); its bytes stay until the purge.
/// The owner, or a family admin when it is shared with their family.
#[utoipa::path(delete, path = "/tracks/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 204, description = "Moved to the trash"), (status = 403, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn delete_track(
    family: FamilyContext,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    crate::trash::move_to_trash(&state, family.viewer(), Some(&headers), crate::db::trash::Kind::MusicTrack, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/music/tracks/{id}/stream — a time-limited URL for the track's audio.
#[utoipa::path(get, path = "/tracks/{id}/stream", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, body = StreamResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn stream_track(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StreamResponse>, AuthError> {
    // find_track applies the visibility rule, so this doubles as the
    // authorization check before a presigned URL is handed out.
    let track = db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let key: String = sqlx::query_scalar(
        "SELECT object_key FROM media_objects WHERE id = $1",
    )
    .bind(track.audio_object_id)
    .fetch_one(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    let url = state
        .storage()
        .presigned_get(&key, STREAM_EXPIRY_SECS)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(StreamResponse {
        url,
        expires_in_secs: STREAM_EXPIRY_SECS,
    }))
}

/// PUT /api/v1/music/tracks/:id/lyrics — owner-only. Lyrics the user typed or
/// pasted themselves.
///
/// Exists because the automatic path finds almost nothing: reading embedded
/// tags is the only lyrics source with no licensing question attached, and
/// ordinary consumer rips very rarely carry a `USLT` frame. Rather than fetch
/// from a lyrics service this product has no licence to redistribute from, this
/// lets the person who owns the music put the words in themselves.
///
/// Whatever arrives is stored verbatim, including LRC timestamps
/// (`[mm:ss.xx] line`) if the user pasted a synced file — the format is the
/// client's to interpret, and normalising it here would throw away timing this
/// backend has no reason to discard.
///
/// An empty body clears the lyrics *and* the source marker, so the track falls
/// back to being read from its file again on the next request.
#[utoipa::path(put, path = "/tracks/{id}/lyrics", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = SetLyricsRequest,
    responses((status = 200, body = LyricsResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn set_track_lyrics(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetLyricsRequest>,
) -> Result<Json<LyricsResponse>, AuthError> {
    db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let lyrics = body.lyrics.unwrap_or_default();
    let trimmed = lyrics.trim();

    db::music::set_user_track_lyrics(state.db(), id, family.user_id, trimmed)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(LyricsResponse {
        lyrics: (!trimmed.is_empty()).then(|| trimmed.to_string()),
    }))
}

/// GET /api/v1/music/tracks/{id}/lyrics — the embedded lyrics tag from the
/// track's own stored audio file (ID3 USLT for MP3, the equivalent generic
/// `ItemKey::Lyrics` item for FLAC/M4A/OGG via `lofty`), read once and
/// cached on `music_tracks.lyrics` (see that column's own doc comment for
/// the `NULL`/`""`/text three-state meaning). Deliberately not folded into
/// `TrackResponse`/`track_to_response` — `GET /tracks` returns the whole
/// library on every call, and lyrics can run to a few KB of text most of
/// those requests have no use for.
#[utoipa::path(get, path = "/tracks/{id}/lyrics", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, body = LyricsResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_track_lyrics(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LyricsResponse>, AuthError> {
    let track = db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if let Some(lyrics) = track.lyrics {
        return Ok(Json(LyricsResponse {
            lyrics: (!lyrics.is_empty()).then_some(lyrics),
        }));
    }

    let key: String = sqlx::query_scalar("SELECT object_key FROM media_objects WHERE id = $1")
        .bind(track.audio_object_id)
        .fetch_one(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    let temp = state.storage().download_to_temp(&key).await.map_err(AuthError::Internal)?;
    let lyrics = parse_lyrics_tag(temp.path()).unwrap_or_default();

    db::music::update_track_lyrics(state.db(), id, &lyrics)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(LyricsResponse {
        lyrics: (!lyrics.is_empty()).then_some(lyrics),
    }))
}

/// The subset of `upload_track`'s own fields that can also come from the file itself, plus a
/// front cover picture if one is embedded. All `None`/empty on any parse failure — same
/// "never fail the request over unreadable tags" reasoning as `parse_lyrics_tag`.
#[derive(Default)]
struct EmbeddedTags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    /// ID3 `TPE2`, MP4 `aART`, Vorbis `ALBUMARTIST`.
    album_artist: Option<String>,
    /// ID3 `TCMP`, MP4 `cpil`, Vorbis `COMPILATION`.
    is_compilation: bool,
    genre: Option<String>,
    track_number: Option<i32>,
    /// Vorbis `DISCNUMBER`/`DISCTOTAL`, ID3 `TPOS`, MP4 `disk`.
    disc_number: Option<i32>,
    disc_total: Option<i32>,
    /// (raw bytes, file extension, content type) — extension and content type both come from
    /// the picture's own declared MIME type, not guessed from bytes.
    cover: Option<(Vec<u8>, &'static str, &'static str)>,
}

/// Reads whatever `upload_track` didn't already get from the multipart form directly off the
/// **already-on-disk** temp file (`audio.temp_file.path()`), before it's ever pushed to storage
/// — no second read of the bytes needed. Confirmed live as the actual gap behind a real bug
/// report: a folder of Queen FLAC albums uploaded through the Mac client (which never reads
/// local tags before uploading) landed with only a filename-derived title and nothing else —
/// artist/album/genre/cover all `NULL`, since `upload_track` previously took them from the
/// multipart form alone. Reuses `lofty`, the same crate `parse_lyrics_tag` already uses, just
/// against the temp file's local path (`Probe::open`) instead of bytes already fetched from
/// storage, and pulling more fields than lyrics-only needs.
///
/// The temp file's path has no extension (`NamedTempFile` names are random), so `Probe::open`'s
/// own extension-based guess never has anything to go on — `guess_file_type()` (content
/// sniffing) is not optional here the way its doc example implies, matching what
/// `parse_lyrics_tag` already does for the same reason.
fn read_embedded_tags(path: &std::path::Path) -> EmbeddedTags {
    use lofty::file::TaggedFileExt;
    use lofty::picture::PictureType;
    use lofty::probe::Probe;
    use lofty::tag::Accessor;

    let Ok(probe) = Probe::open(path) else { return EmbeddedTags::default(); };
    let Ok(probe) = probe.guess_file_type() else { return EmbeddedTags::default(); };
    let Ok(tagged_file) = probe.read() else { return EmbeddedTags::default(); };
    let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) else {
        return EmbeddedTags::default();
    };

    fn non_empty(value: Option<std::borrow::Cow<'_, str>>) -> Option<String> {
        value.map(|c| c.into_owned()).filter(|s| !s.trim().is_empty())
    }

    let cover = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .map(|p| {
            let (ext, content_type) = match p.mime_type().map(|m| m.as_str()) {
                Some("image/png") => ("png", "image/png"),
                Some("image/webp") => ("webp", "image/webp"),
                _ => ("jpg", "image/jpeg"),
            };
            (p.data().to_vec(), ext, content_type)
        });

    let album_artist = tag
        .get_string(lofty::tag::ItemKey::AlbumArtist)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let is_compilation = tag
        .get_string(lofty::tag::ItemKey::FlagCompilation)
        .is_some_and(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "True"));

    EmbeddedTags {
        title: non_empty(tag.title()),
        artist: non_empty(tag.artist()),
        album: non_empty(tag.album()),
        album_artist,
        is_compilation,
        genre: non_empty(tag.genre()),
        track_number: tag.track().map(|n| n as i32),
        disc_number: tag.disk().map(|n| n as i32),
        disc_total: tag.disk_total().map(|n| n as i32),
        cover,
    }
}

/// `None` on any parse failure (an unrecognized/corrupt file, or a codec
/// `lofty` doesn't support) rather than propagating an error — a track with
/// unreadable tags should read back as "no lyrics found", not fail the
/// whole request, and it still gets cached as `""` by the caller so a
/// broken file isn't re-parsed on every subsequent request either.
fn parse_lyrics_tag(path: &std::path::Path) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::ItemKey;

    let tagged_file = Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()?;
    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag())?;

    // ID3v2's USLT frame (by far the most common real-world "lyrics" tag,
    // written by MP3 taggers) maps to `UnsyncLyrics`, not `Lyrics` — `Lyrics`
    // is what a Vorbis Comment "LYRICS" field (FLAC/OGG) maps to instead.
    // Checked live against a real MP3 with an embedded USLT frame while
    // writing this: `ItemKey::Lyrics` alone silently found nothing.
    tag.get_string(ItemKey::UnsyncLyrics)
        .or_else(|| tag.get_string(ItemKey::Lyrics))
        .map(str::to_string)
}

/// GET /api/v1/music/tracks/{id}/cover — the track's cover image.
#[utoipa::path(get, path = "/tracks/{id}/cover", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, description = "The image bytes", content_type = "image/*"), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AuthError> {
    db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT mo.object_key, mo.content_type
         FROM media_objects mo
         JOIN music_tracks mt ON mt.cover_object_id = mo.id
         WHERE mt.id = $1",
    )
    .bind(id)
    .fetch_optional(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    .ok_or(AuthError::ItemNotFound)?;

    let bytes = state.storage().get(&row.0).await.map_err(AuthError::Internal)?;
    Ok(([(header::CONTENT_TYPE, row.1)], Body::from(bytes)).into_response())
}

/// POST /api/v1/music/tracks/{id}/upload-cover — replace the track's cover. Owner only.
#[utoipa::path(post, path = "/tracks/{id}/upload-cover", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body(content_type = "multipart/form-data", description = "fields: `cover` (the image file)"),
    responses((status = 201, description = "Stored"), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn upload_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    db::music::find_track_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let mut upload: Option<TempUpload> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        if field.name().unwrap_or_default() == "cover" {
            upload = Some(save_field_to_temp(field).await?);
        }
    }

    let upload = upload.ok_or_else(|| AuthError::BadRequest("missing cover payload".to_string()))?;
    let ext = file_extension(&upload.file_name).unwrap_or("jpg");
    let object_key = crate::storage::family_key(
        family.family_id,
        format!("music/{}/covers/{}.{ext}", family.user_id, id),
    );

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;

    sqlx::query(
        "UPDATE music_tracks SET cover_object_id = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $1",
    )
    .bind(id)
    .bind(media_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(StatusCode::CREATED)
}

// ── Track progress ────────────────────────────────────────────────────────

// ── Smart playlists ───────────────────────────────────────────────────────
//
// A saved query, not a list of tracks. See
// docs/music-signals-and-smart-playlists-plan.md §4 and migration 0074.

use crate::music::rules::Rule;

/// How many candidates the database may return before sequencing. Generous
/// against MAX_TRACKS so the sequencer has room to satisfy the per-artist and
/// arc constraints; without slack it would run out of eligible tracks and stop
/// short of the requested length.
const CANDIDATE_POOL: i64 = 4_000;

#[derive(Serialize, ToSchema)]
pub struct SmartPlaylistResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub rule: serde_json::Value,
    pub mode: String,
    pub shared: bool,
    pub owned: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SmartPlaylistRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[schema(value_type = Object)]
    pub rule: Rule,
    /// Share with the family. Sharing a *rule* means the receiver's own history
    /// is what it evaluates against — see §4.4 — so a client must say so.
    #[serde(default)]
    pub shared: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct ResolveRuleRequest {
    #[schema(value_type = Object)]
    pub rule: Rule,
}

#[derive(Serialize, ToSchema)]
pub struct ResolvedPlaylistResponse {
    pub tracks: Vec<TrackResponse>,
    pub total_secs: i64,
    /// How many tracks matched before sequencing. A small number here with a
    /// short playlist means the rule is too narrow, not that the sequencer
    /// failed — worth showing rather than leaving a client to guess.
    pub candidates: usize,
}

fn smart_playlist_response(p: db::music::SmartPlaylist, viewer: Uuid) -> SmartPlaylistResponse {
    SmartPlaylistResponse {
        id: p.id.to_string(),
        name: p.name,
        description: p.description,
        rule: p.rule,
        mode: p.mode,
        shared: p.family_id.is_some(),
        owned: p.user_id == viewer,
        created_at: p.created_at.to_rfc3339(),
        updated_at: p.updated_at.to_rfc3339(),
    }
}

/// GET /music/smart-playlists/presets
///
/// Built-ins rather than rows: they are the same for everyone, and a row per
/// user per preset would be a migration plus a backfill for no gain. A client
/// that wants to edit one POSTs it as a new playlist.
#[utoipa::path(get, path = "/smart-playlists/presets", tag = "music",
    responses((status = 200, description = "Built-in rules, each `{slug, name, rule}`", body = Vec<Object>)))]
async fn list_presets() -> Json<Vec<serde_json::Value>> {
    Json(
        crate::music::rules::presets()
            .into_iter()
            .map(|(slug, name, rule)| {
                serde_json::json!({
                    "slug": slug,
                    "name": name,
                    "rule": rule,
                })
            })
            .collect(),
    )
}

/// GET /api/v1/music/smart-playlists — the caller's smart playlists and those shared with their family.
#[utoipa::path(get, path = "/smart-playlists", tag = "music", security(("bearer" = [])),
    responses((status = 200, body = Vec<SmartPlaylistResponse>)))]
async fn list_smart_playlists(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<SmartPlaylistResponse>>, AuthError> {
    let rows = db::music::list_smart_playlists(state.db(), family.user_id, family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|p| smart_playlist_response(p, family.user_id))
            .collect(),
    ))
}

/// POST /api/v1/music/smart-playlists — save a rule as a smart playlist.
#[utoipa::path(post, path = "/smart-playlists", tag = "music", security(("bearer" = [])),
    request_body = SmartPlaylistRequest,
    responses((status = 201, body = SmartPlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn create_smart_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<SmartPlaylistRequest>,
) -> Result<(StatusCode, Json<SmartPlaylistResponse>), AuthError> {
    body.rule
        .validate()
        .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let rule = serde_json::to_value(&body.rule).map_err(|e| AuthError::Internal(e.into()))?;
    let p = db::music::create_smart_playlist(
        state.db(),
        family.user_id,
        body.shared.then_some(family.family_id),
        body.name.trim(),
        body.description.as_deref(),
        &rule,
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok((
        StatusCode::CREATED,
        Json(smart_playlist_response(p, family.user_id)),
    ))
}

/// GET /api/v1/music/smart-playlists/{id} — one smart playlist.
#[utoipa::path(get, path = "/smart-playlists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Smart playlist id")),
    responses((status = 200, body = SmartPlaylistResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_smart_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SmartPlaylistResponse>, AuthError> {
    let p = db::music::find_smart_playlist(state.db(), id, family.user_id, family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    Ok(Json(smart_playlist_response(p, family.user_id)))
}

/// PUT /api/v1/music/smart-playlists/{id} — replace a smart playlist's name, description and rule. Owner only.
#[utoipa::path(put, path = "/smart-playlists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Smart playlist id")),
    request_body = SmartPlaylistRequest,
    responses((status = 200, body = SmartPlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn update_smart_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SmartPlaylistRequest>,
) -> Result<Json<SmartPlaylistResponse>, AuthError> {
    body.rule
        .validate()
        .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let rule = serde_json::to_value(&body.rule).map_err(|e| AuthError::Internal(e.into()))?;
    let p = db::music::update_smart_playlist(
        state.db(),
        id,
        family.user_id,
        body.name.trim(),
        body.description.as_deref(),
        &rule,
    )
    .await
    .map_err(AuthError::Internal)?
    .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(smart_playlist_response(p, family.user_id)))
}

/// DELETE /api/v1/music/smart-playlists/{id} — delete a smart playlist. Owner only.
#[utoipa::path(delete, path = "/smart-playlists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Smart playlist id")),
    responses((status = 204, description = "Deleted"), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn delete_smart_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    if db::music::delete_smart_playlist(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::ItemNotFound)
    }
}

/// POST /music/smart-playlists/{id}/resolve
///
/// Evaluated against **the caller's** history, not the owner's. That is the
/// defining property of sharing a rule rather than a result (§4.4): "unplayed
/// 80s rock" is a different set for two people, and a client must present it
/// that way or it reads as a bug.
#[utoipa::path(post, path = "/smart-playlists/{id}/resolve", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Smart playlist id")),
    responses((status = 200, body = ResolvedPlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn resolve_smart_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ResolvedPlaylistResponse>, AuthError> {
    let p = db::music::find_smart_playlist(state.db(), id, family.user_id, family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let rule: Rule = serde_json::from_value(p.rule)
        .map_err(|e| AuthError::BadRequest(format!("stored rule is not valid: {e}")))?;

    resolve(&state, &family, &rule).await.map(Json)
}

/// POST /music/smart-playlists/resolve
///
/// Run a rule without saving it. This is what a natural-language request lands
/// on (§5): the model produces a rule, the caller resolves it, and only if the
/// result is wanted does it become a playlist.
#[utoipa::path(post, path = "/smart-playlists/resolve", tag = "music", security(("bearer" = [])),
    request_body = ResolveRuleRequest,
    responses((status = 200, body = ResolvedPlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn resolve_rule(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<ResolveRuleRequest>,
) -> Result<Json<ResolvedPlaylistResponse>, AuthError> {
    body.rule
        .validate()
        .map_err(|e| AuthError::BadRequest(e.to_string()))?;
    resolve(&state, &family, &body.rule).await.map(Json)
}

#[derive(Deserialize, ToSchema)]
pub struct IntentRequest {
    pub text: String,
}

#[derive(Serialize, ToSchema)]
pub struct IntentResponse {
    /// The rule, ready to POST to `/smart-playlists/resolve`. **Not tracks** —
    /// see music::intent for why that boundary matters.
    #[schema(value_type = Object)]
    pub rule: crate::music::rules::Rule,
}

/// POST /music/intent
///
/// Text in, validated rule out. The caller resolves it; nothing is saved and
/// nothing is played until they ask.
///
/// Apple clients should prefer their on-device model and use this only as a
/// fallback. What leaves the device either way is the sentence and the
/// library's genre vocabulary — never track titles, never listening history.
#[utoipa::path(post, path = "/intent", tag = "music", security(("bearer" = [])),
    request_body = IntentRequest,
    responses((status = 200, body = IntentResponse), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn parse_intent(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<IntentRequest>,
) -> Result<Json<IntentResponse>, AuthError> {
    let text = body.text.trim();
    if text.is_empty() {
        return Err(AuthError::BadRequest("text must not be empty".into()));
    }
    if text.len() > 500 {
        return Err(AuthError::BadRequest("text is too long".into()));
    }

    // The library's own genre vocabulary, so "some jazz" matches what this
    // library calls jazz rather than a guess.
    let genres: Vec<String> = db::music::list_genres(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .into_iter()
        .map(|(name, _count)| name)
        .collect();

    let rule = crate::music::intent::parse(text, &genres)
        .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    Ok(Json(IntentResponse { rule }))
}

#[derive(Deserialize, ToSchema)]
pub struct FreezeRequest {
    /// Defaults to the smart playlist's own name.
    #[serde(default)]
    pub name: Option<String>,
    /// Share the resulting playlist with the family.
    #[serde(default)]
    pub shared: bool,
    /// Mark the source as frozen. False makes this "send a copy" — the rule
    /// keeps living and the copy goes its own way, which is the third of the
    /// three sharing modes in §4.4.
    #[serde(default = "default_true")]
    pub mark_source: bool,
}

/// POST /music/smart-playlists/{id}/freeze
///
/// Materialise a rule into an ordinary playlist: resolve it once, write the
/// result as `music_playlists` rows, and stop re-evaluating.
///
/// This is the difference between the three sharing modes (§4.4) made concrete.
/// Sharing a *rule* means the receiver's own history decides what it contains,
/// which is right for a tool and wrong for "listen to what I made for you".
/// Freezing gives the second: a fixed list that can be edited and that never
/// changes under anyone. A single "share" button doing one of these silently
/// would be the wrong design.
#[utoipa::path(post, path = "/smart-playlists/{id}/freeze", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Smart playlist id")),
    request_body = FreezeRequest,
    responses((status = 201, body = PlaylistResponse), (status = 400, description = "The rule matches nothing", body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn freeze_smart_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<FreezeRequest>,
) -> Result<(StatusCode, Json<PlaylistResponse>), AuthError> {
    let sp = db::music::find_smart_playlist(state.db(), id, family.user_id, family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let rule: Rule = serde_json::from_value(sp.rule.clone())
        .map_err(|e| AuthError::BadRequest(format!("stored rule is not valid: {e}")))?;

    let resolved = resolve(&state, &family, &rule).await?;
    if resolved.tracks.is_empty() {
        return Err(AuthError::BadRequest(
            "the rule matches nothing right now, so there is nothing to freeze".into(),
        ));
    }

    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(&sp.name);

    let playlist = db::music::insert_playlist(
        state.db(),
        family.user_id,
        body.shared.then_some(family.family_id),
        name,
        sp.description.as_deref(),
        true,
    )
    .await
    .map_err(AuthError::Internal)?;

    // Order matters — the sequence is the whole point of resolving — and
    // add_track_to_playlist appends, so inserting in order preserves it.
    for t in &resolved.tracks {
        let track_id: Uuid = t
            .id
            .parse()
            .map_err(|e| AuthError::Internal(anyhow::anyhow!("bad track id: {e}")))?;
        db::music::add_track_to_playlist(state.db(), playlist.id, track_id)
            .await
            .map_err(AuthError::Internal)?;
    }

    // Only the owner can mark their own source frozen; a member who froze a
    // shared rule gets their copy and leaves the original alone.
    if body.mark_source && sp.user_id == family.user_id {
        db::music::mark_smart_playlist_frozen(state.db(), id, family.user_id, playlist.id)
            .await
            .map_err(AuthError::Internal)?;
    }

    Ok((
        StatusCode::CREATED,
        Json(playlist_to_response(
            playlist,
            resolved.tracks.len() as i64,
            false,
            family.user_id,
        )),
    ))
}

async fn resolve(
    state: &AppState,
    family: &FamilyContext,
    rule: &Rule,
) -> Result<ResolvedPlaylistResponse, AuthError> {
    let candidates =
        db::music::smart_playlist_candidates(state.db(), family.viewer(), rule, CANDIDATE_POOL)
            .await
            .map_err(AuthError::Internal)?;

    let seed = rand::random::<u64>();
    let ids = crate::music::sequence::build(&candidates, rule, seed);

    // One lookup for the whole result, then reordered in memory: the sequence
    // is the point, and an ORDER BY cannot express it.
    let mut tracks = Vec::with_capacity(ids.len());
    let mut total_secs: i64 = 0;
    for id in &ids {
        if let Some(t) = db::music::find_track(state.db(), *id, family.viewer())
            .await
            .map_err(AuthError::Internal)?
        {
            total_secs += t.duration_secs.unwrap_or(0) as i64;
            tracks.push(track_to_response(t, family.user_id));
        }
    }

    Ok(ResolvedPlaylistResponse {
        tracks,
        total_secs,
        candidates: candidates.len(),
    })
}

// ── Negative feedback (dislike / never play) ──────────────────────────────
//
// Two meanings, not one: a dislike recovers over months, a ban does not until
// undone. Neither touches the track — see migration 0069.

#[derive(Deserialize, ToSchema)]
pub struct TrackFeedbackRequest {
    /// `dislike` or `banned`.
    pub kind: String,
}

#[derive(Serialize, ToSchema)]
pub struct TrackFeedbackResponse {
    pub track_id: String,
    pub kind: String,
    pub updated_at: String,
}

/// PUT /music/tracks/{id}/feedback
/// Mark a track disliked or banned for the caller.
#[utoipa::path(put, path = "/tracks/{id}/feedback", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = TrackFeedbackRequest,
    responses((status = 204, description = "Stored"), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn set_track_feedback(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<TrackFeedbackRequest>,
) -> Result<StatusCode, AuthError> {
    let kind = body.kind.trim();
    if !matches!(kind, "dislike" | "banned") {
        return Err(AuthError::BadRequest(
            "kind must be 'dislike' or 'banned'".into(),
        ));
    }

    // find_track applies the visibility rule, so this doubles as the
    // authorization check: feedback on a track you cannot see is a 404, not a
    // silently stored row.
    //
    // ItemNotFound, not NotFound — the latter means "no such user" and maps to
    // 401 (auth::error). Several older music handlers use it for a missing
    // track and answer 401 where they mean 404; do not copy that here.
    db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::music::set_track_feedback(state.db(), family.user_id, id, kind)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /music/tracks/{id}/feedback
///
/// Idempotent: clearing feedback that was never set is success, not 404. The
/// caller wants the track unmarked, and it is.
#[utoipa::path(delete, path = "/tracks/{id}/feedback", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 204, description = "Cleared")))]
async fn clear_track_feedback(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::music::clear_track_feedback(state.db(), family.user_id, id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /music/feedback?kind=banned
///
/// The undo list. Someone who banned a track by a mis-tap has no other way to
/// find it, because nothing about the track itself looks different.
#[utoipa::path(get, path = "/feedback", tag = "music", security(("bearer" = [])),
    params(TrackFeedbackQuery),
    responses((status = 200, body = Vec<TrackFeedbackResponse>), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn list_track_feedback(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<TrackFeedbackQuery>,
) -> Result<Json<Vec<TrackFeedbackResponse>>, AuthError> {
    if let Some(k) = params.kind.as_deref() {
        if !matches!(k, "dislike" | "banned") {
            return Err(AuthError::BadRequest(
                "kind must be 'dislike' or 'banned'".into(),
            ));
        }
    }

    let rows = db::music::list_track_feedback(state.db(), family.user_id, params.kind.as_deref())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|r| TrackFeedbackResponse {
                track_id: r.track_id.to_string(),
                kind: r.kind,
                updated_at: r.updated_at.to_rfc3339(),
            })
            .collect(),
    ))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TrackFeedbackQuery {
    #[serde(default)]
    pub kind: Option<String>,
}

/// GET /api/v1/music/tracks/{id}/progress — the caller's playback position in a track.
#[utoipa::path(get, path = "/tracks/{id}/progress", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 200, body = ProgressResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_track_progress(
    auth: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProgressResponse>, AuthError> {
    let progress = db::music::get_track_progress(state.db(), auth.user_id, id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(ProgressResponse {
        track_id: progress.track_id.to_string(),
        position_secs: progress.position_secs,
        completed: progress.completed,
        updated_at: progress.updated_at.to_rfc3339(),
    }))
}

/// PUT /api/v1/music/tracks/{id}/progress — save the caller's playback position in a track.
#[utoipa::path(put, path = "/tracks/{id}/progress", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    request_body = UpsertProgressRequest,
    responses((status = 200, body = ProgressResponse)))]
async fn upsert_track_progress(
    auth: crate::auth::middleware::AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertProgressRequest>,
) -> Result<Json<ProgressResponse>, AuthError> {
    let previous = db::music::get_track_progress(state.db(), auth.user_id, id)
        .await
        .map_err(AuthError::Internal)?
        .map(|p| p.position_secs)
        .unwrap_or(0.0);

    let progress = db::music::upsert_track_progress(
        state.db(),
        auth.user_id,
        id,
        body.position_secs,
        body.completed,
    )
    .await
    .map_err(AuthError::Internal)?;

    let _ = db::stats::derive_from_progress(
        state.db(),
        auth.user_id,
        access::MUSIC,
        id,
        None,
        body.position_secs - previous,
        "other",
    )
    .await;

    Ok(Json(ProgressResponse {
        track_id: progress.track_id.to_string(),
        position_secs: progress.position_secs,
        completed: progress.completed,
        updated_at: progress.updated_at.to_rfc3339(),
    }))
}

// ── Playlist handlers ─────────────────────────────────────────────────────

/// GET /api/v1/music/playlists — every playlist the caller can see.
#[utoipa::path(get, path = "/playlists", tag = "music", security(("bearer" = [])),
    responses((status = 200, body = Vec<PlaylistResponse>)))]
async fn list_playlists(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<PlaylistResponse>>, AuthError> {
    let playlists = db::music::list_playlists(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::with_capacity(playlists.len());
    for p in playlists {
        let (count, has_track_cover) = playlist_contents(&state, p.id).await?;
        responses.push(playlist_to_response(p, count, has_track_cover, family.user_id));
    }

    Ok(Json(responses))
}

/// POST /api/v1/music/playlists — create a playlist, optionally with its tracks.
#[utoipa::path(post, path = "/playlists", tag = "music", security(("bearer" = [])),
    request_body = CreatePlaylistRequest,
    responses((status = 201, body = PlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody)))]
async fn create_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreatePlaylistRequest>,
) -> Result<(StatusCode, Json<PlaylistResponse>), AuthError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AuthError::BadRequest("name is required".to_string()));
    }
    if body.track_ids.len() > MAX_PLAYLIST_CREATE_TRACKS {
        return Err(AuthError::BadRequest(format!(
            "at most {MAX_PLAYLIST_CREATE_TRACKS} tracks in one request"
        )));
    }

    let family_id =
        access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let playlist = db::music::insert_playlist(
        state.db(),
        family.user_id,
        family_id,
        name,
        trim_opt(body.description.as_deref()),
        body.generated,
    )
    .await
    .map_err(AuthError::Internal)?;

    let added = if body.track_ids.is_empty() {
        0
    } else {
        db::music::add_tracks_to_playlist(state.db(), playlist.id, &body.track_ids, family.viewer())
            .await
            .map_err(AuthError::Internal)?
    };

    Ok((
        StatusCode::CREATED,
        Json(playlist_to_response(playlist, added as i64, false, family.user_id)),
    ))
}

/// GET /api/v1/music/starred — ids of the songs this user starred.
///
/// Stars were reachable only through the Subsonic API until the web's player
/// got a heart; "Forgotten favourites" reads them, so without a way to set
/// them in our own clients it could never find anything.
#[utoipa::path(get, path = "/starred", tag = "music", security(("bearer" = [])),
    responses((status = 200, description = "Track ids", body = Vec<String>)))]
async fn list_starred_tracks(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, AuthError> {
    let starred = db::music::starred_track_ids(state.db(), family.user_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(starred.into_iter().map(|(id, _)| id.to_string()).collect()))
}

/// PUT /api/v1/music/tracks/{id}/star — star a song the caller can play. Idempotent.
#[utoipa::path(put, path = "/tracks/{id}/star", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 204, description = "Starred"), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn star_track(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::music::find_track(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    db::music::star_track(state.db(), family.user_id, id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/music/tracks/{id}/star — idempotent.
#[utoipa::path(delete, path = "/tracks/{id}/star", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Track id")),
    responses((status = 204, description = "Unstarred")))]
async fn unstar_track(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::music::unstar_track(state.db(), family.user_id, id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct PlaylistAudienceEntry {
    pub user_id: String,
    pub display_name: String,
    pub display_label: Option<String>,
    /// Can play the owner's live playlist now.
    pub can_listen: bool,
    /// A family admin, who sees everything shared with the family whatever is chosen here.
    pub locked: bool,
}

/// GET /api/v1/music/playlists/{id}/audience — the owner asks which family members
/// can play their playlist. Everyone but the owner; all `false` while it is private.
#[utoipa::path(get, path = "/playlists/{id}/audience", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    responses((status = 200, body = Vec<PlaylistAudienceEntry>), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn playlist_audience(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PlaylistAudienceEntry>>, AuthError> {
    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let audience =
        access::item_audience_in(state.db(), family.family_id, access::MUSIC, "music_playlists", id)
            .await
            .map_err(AuthError::Internal)?;
    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(
        members
            .iter()
            .filter(|m| m.user_id != family.user_id)
            .map(|m| {
                let (can_listen, _) = audience
                    .iter()
                    .find(|(u, _, _)| *u == m.user_id)
                    .map(|(_, can, locked)| (*can, *locked))
                    .unwrap_or((false, false));
                PlaylistAudienceEntry {
                    user_id: m.user_id.to_string(),
                    display_name: m.display_name.clone(),
                    display_label: m.display_label.clone(),
                    can_listen,
                    locked: m.role == "family_admin",
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct SharePlaylistRequest {
    /// `live` — the chosen members play the owner's playlist and see every change;
    /// `copy` — each of them gets a playlist of their own with the same songs.
    pub mode: String,
    pub user_ids: Vec<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct SharePlaylistResponse {
    /// The owner's private songs that were shared with the chosen members so they can play them.
    pub shared_tracks: usize,
    /// Copies made (`copy` mode).
    pub copies: usize,
}

/// PUT /api/v1/music/playlists/{id}/share — the owner shares a playlist with chosen
/// family members, live or as copies.
///
/// A playlist is only as playable as its songs: a song only its owner can hear would be
/// a silent gap for everyone else. So the owner's private songs in it are shared first —
/// with the chosen members only, not the whole family (the user's call, 2026-09-30) — and
/// an owner's song already shared with someone else is opened to the chosen members too.
/// Songs other people own are theirs to share and are left alone.
///
/// `live` with nobody chosen makes the playlist private again. Family admins see anything
/// shared with the family whatever is chosen, which the audience reports as `locked`.
#[utoipa::path(put, path = "/playlists/{id}/share", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    request_body = SharePlaylistRequest,
    responses((status = 200, body = SharePlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn share_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SharePlaylistRequest>,
) -> Result<Json<SharePlaylistResponse>, AuthError> {
    let live = match body.mode.as_str() {
        "live" => true,
        "copy" => false,
        _ => return Err(AuthError::BadRequest("mode must be 'live' or 'copy'".into())),
    };
    let playlist = db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let wanted: std::collections::HashSet<Uuid> =
        body.user_ids.into_iter().filter(|u| *u != family.user_id).collect();
    if let Some(stranger) = wanted.iter().find(|u| !members.iter().any(|m| m.user_id == **u)) {
        return Err(AuthError::BadRequest(format!("{stranger} is not a member of this family")));
    }
    if !live && wanted.is_empty() {
        return Err(AuthError::BadRequest("choose who gets a copy".into()));
    }

    // Owner and admins are decided by the access rule itself; a row about them is never read.
    let decisions: Vec<(Uuid, bool)> = members
        .iter()
        .filter(|m| m.user_id != family.user_id && m.role != "family_admin")
        .map(|m| (m.user_id, wanted.contains(&m.user_id)))
        .collect();

    let tracks = db::music::playlist_track_owners(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    let mut shared_tracks = 0;
    if !wanted.is_empty() {
        let mut done = std::collections::HashSet::new();
        for (track_id, owner, track_family) in &tracks {
            if *owner != family.user_id || !done.insert(*track_id) {
                continue;
            }
            if track_family.is_none() {
                // Only the owner could play it: share it with the chosen members, nobody else.
                access::set_shared(state.db(), family.user_id, family.family_id, access::MUSIC, *track_id, true)
                    .await
                    .map_err(AuthError::Internal)?;
                access::set_item_audience(state.db(), family.family_id, access::MUSIC, *track_id, &decisions, family.user_id)
                    .await
                    .map_err(AuthError::Internal)?;
                shared_tracks += 1;
                continue;
            }
            // Already shared, maybe not with everyone chosen (an earlier share to someone else
            // left them a deny): add the chosen members and leave everyone else as they were.
            let audience = access::item_audience(state.db(), family.family_id, access::MUSIC, *track_id)
                .await
                .map_err(AuthError::Internal)?;
            let missing = audience.iter().any(|(u, can, _)| !can && wanted.contains(u));
            if !missing {
                continue;
            }
            let widened: Vec<(Uuid, bool)> = decisions
                .iter()
                .map(|(u, _)| {
                    let can_now = audience.iter().any(|(a, can, _)| a == u && *can);
                    (*u, can_now || wanted.contains(u))
                })
                .collect();
            access::set_item_audience(state.db(), family.family_id, access::MUSIC, *track_id, &widened, family.user_id)
                .await
                .map_err(AuthError::Internal)?;
            shared_tracks += 1;
        }
    }

    let owner_name = members
        .iter()
        .find(|m| m.user_id == family.user_id)
        .map(|m| m.display_name.clone())
        .unwrap_or_default();
    let mut copies = 0;

    if live {
        // Who could play it before, so only the newly added are told about it.
        let before: std::collections::HashSet<Uuid> = if playlist.family_id.is_some() {
            access::item_audience_in(state.db(), family.family_id, access::MUSIC, "music_playlists", id)
                .await
                .map_err(AuthError::Internal)?
                .into_iter()
                .filter(|(_, can, _)| *can)
                .map(|(u, _, _)| u)
                .collect()
        } else {
            Default::default()
        };

        sqlx::query("UPDATE music_playlists SET family_id = $2, updated_at = now() WHERE id = $1 AND user_id = $3")
            .bind(id)
            .bind((!wanted.is_empty()).then_some(family.family_id))
            .bind(family.user_id)
            .execute(state.db())
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;

        if !wanted.is_empty() {
            access::set_item_audience_in(
                state.db(),
                family.family_id,
                access::MUSIC,
                "music_playlists",
                id,
                &decisions,
                family.user_id,
            )
            .await
            .map_err(AuthError::Internal)?;
        }

        for user_id in wanted.iter().filter(|u| !before.contains(u)) {
            let data = serde_json::json!({ "playlist_id": id.to_string(), "from": owner_name, "name": playlist.name, "mode": "live" });
            let _ = db::sync::queue_notification(
                state.db(),
                *user_id,
                "playlist_shared",
                &format!("{owner_name} shared a playlist with you"),
                Some(&playlist.name),
                Some(&data),
            )
            .await;
        }
    } else {
        let track_ids: Vec<Uuid> = tracks.iter().map(|(t, _, _)| *t).collect();
        for member in members.iter().filter(|m| wanted.contains(&m.user_id)) {
            let viewer = access::Viewer {
                user_id: member.user_id,
                family_id: family.family_id,
                is_family_admin: member.role == "family_admin",
            };
            let copy = db::music::insert_playlist(
                state.db(),
                member.user_id,
                None,
                &playlist.name,
                playlist.description.as_deref(),
                false,
            )
            .await
            .map_err(AuthError::Internal)?;
            db::music::add_tracks_to_playlist(state.db(), copy.id, &track_ids, viewer)
                .await
                .map_err(AuthError::Internal)?;
            copies += 1;

            let data = serde_json::json!({ "playlist_id": copy.id.to_string(), "from": owner_name, "name": playlist.name, "mode": "copy" });
            let _ = db::sync::queue_notification(
                state.db(),
                member.user_id,
                "playlist_shared",
                &format!("{owner_name} sent you a playlist"),
                Some(&playlist.name),
                Some(&data),
            )
            .await;
        }
    }

    Ok(Json(SharePlaylistResponse { shared_tracks, copies }))
}

/// Tracks accepted with `POST /music/playlists`. A generated playlist is 50–100
/// songs; the cap only keeps one request bounded.
const MAX_PLAYLIST_CREATE_TRACKS: usize = 1000;

/// PUT /api/v1/music/playlists/{id}/keep — the owner keeps a generated playlist.
/// 404 for a playlist that is not theirs or was not generated.
#[utoipa::path(put, path = "/playlists/{id}/keep", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    responses((status = 204, description = "Kept"), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn keep_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    if db::music::keep_playlist(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::ItemNotFound)
    }
}

/// GET /api/v1/music/playlists/{id} — one playlist the caller can see.
#[utoipa::path(get, path = "/playlists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    responses((status = 200, body = PlaylistResponse), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn get_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PlaylistResponse>, AuthError> {
    let playlist = db::music::find_playlist(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let (count, has_track_cover) = playlist_contents(&state, id).await?;

    Ok(Json(playlist_to_response(playlist, count, has_track_cover, family.user_id)))
}

/// PUT /api/v1/music/playlists/:id/visibility — owner only.
#[utoipa::path(put, path = "/playlists/{id}/visibility", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    request_body = SetVisibilityRequest,
    responses((status = 204, description = "Changed"), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn set_playlist_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetVisibilityRequest>,
) -> Result<StatusCode, AuthError> {
    let shared = parse_visibility(&body.visibility)?;

    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    sqlx::query("UPDATE music_playlists SET family_id = $2 WHERE id = $1 AND user_id = $3")
        .bind(id)
        .bind(shared.then_some(family.family_id))
        .bind(family.user_id)
        .execute(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/v1/music/playlists/{id} — rename a playlist or change its description. Owner only.
#[utoipa::path(put, path = "/playlists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    request_body = UpdatePlaylistRequest,
    responses((status = 200, body = PlaylistResponse), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn update_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePlaylistRequest>,
) -> Result<Json<PlaylistResponse>, AuthError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AuthError::BadRequest("name is required".to_string()));
    }

    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::music::update_playlist(
        state.db(),
        id,
        family.user_id,
        name,
        trim_opt(body.description.as_deref()),
    )
    .await
    .map_err(AuthError::Internal)?;

    let fresh = db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let (count, has_track_cover) = playlist_contents(&state, id).await?;

    Ok(Json(playlist_to_response(fresh, count, has_track_cover, family.user_id)))
}

/// Moves the playlist to the trash (30 days).
#[utoipa::path(delete, path = "/playlists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    responses((status = 204, description = "Moved to the trash"), (status = 403, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn delete_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    crate::trash::move_to_trash(&state, family.viewer(), Some(&headers), crate::db::trash::Kind::Playlist, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/music/playlists/{id}/tracks — the playlist's entries in order, skipping tracks the caller may not play.
#[utoipa::path(get, path = "/playlists/{id}/tracks", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    responses((status = 200, body = Vec<PlaylistTrackResponse>), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn list_playlist_tracks(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PlaylistTrackResponse>>, AuthError> {
    db::music::find_playlist(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let entries = db::music::list_playlist_tracks(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    // Entries the viewer may not play are skipped rather than erroring: a
    // shared playlist can legitimately reference tracks a restricted member
    // is not granted.
    let mut result = Vec::with_capacity(entries.len());
    for entry in entries {
        let track = db::music::find_track(state.db(), entry.track_id, family.viewer())
            .await
            .map_err(AuthError::Internal)?;
        if let Some(track) = track {
            result.push(PlaylistTrackResponse {
                entry_id: entry.id.to_string(),
                position: entry.position,
                track: track_to_response(track, family.user_id),
            });
        }
    }

    Ok(Json(result))
}

/// POST /api/v1/music/playlists/{id}/tracks — append a track the caller can play. Owner only.
#[utoipa::path(post, path = "/playlists/{id}/tracks", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    request_body = AddTrackToPlaylistRequest,
    responses((status = 201, description = "Added"), (status = 400, description = "Bad or unknown track id", body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn add_to_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AddTrackToPlaylistRequest>,
) -> Result<StatusCode, AuthError> {
    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let track_id: Uuid = body
        .track_id
        .parse()
        .map_err(|e| AuthError::BadRequest(format!("invalid track_id: {e}")))?;

    // Any track the caller can play may go into their playlist, including
    // family-shared tracks owned by someone else.
    db::music::find_track(state.db(), track_id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("track not found".to_string()))?;

    db::music::add_track_to_playlist(state.db(), id, track_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::CREATED)
}

/// DELETE /api/v1/music/playlists/{id}/tracks/{entry_id} — remove one entry. Owner only.
#[utoipa::path(delete, path = "/playlists/{id}/tracks/{entry_id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id"), ("entry_id" = Uuid, Path, description = "Playlist entry id")),
    responses((status = 204, description = "Removed"), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn remove_from_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((id, entry_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AuthError> {
    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::music::remove_track_from_playlist(state.db(), id, entry_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/v1/music/playlists/{id}/tracks/reorder — set the order of the playlist's entries. Owner only.
#[utoipa::path(put, path = "/playlists/{id}/tracks/reorder", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    request_body = ReorderPlaylistRequest,
    responses((status = 204, description = "Reordered"), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn reorder_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReorderPlaylistRequest>,
) -> Result<StatusCode, AuthError> {
    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let entry_ids: Vec<Uuid> = body
        .entry_ids
        .iter()
        .map(|s| s.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AuthError::BadRequest(format!("invalid entry_id: {e}")))?;

    db::music::reorder_playlist_tracks(state.db(), id, &entry_ids)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/music/playlists/{id}/cover — the playlist's cover, or the first of its tracks' covers.
#[utoipa::path(get, path = "/playlists/{id}/cover", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    responses((status = 200, description = "The image bytes", content_type = "image/*"), (status = 404, description = "No playlist, or neither it nor its tracks have a cover", body = crate::http::openapi::ErrorBody)))]
async fn get_playlist_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AuthError> {
    db::music::find_playlist(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    // The playlist's own cover if one was uploaded, otherwise the first track in it that has
    // artwork. Almost no playlist gets a cover uploaded, and every client was left drawing a
    // grey placeholder over a list of songs that all have covers of their own.
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT object_key, content_type FROM (
             SELECT mo.object_key, mo.content_type, 0 AS tier, 0 AS position
             FROM media_objects mo
             JOIN music_playlists mp ON mp.cover_object_id = mo.id
             WHERE mp.id = $1
             UNION ALL
             SELECT mo.object_key, mo.content_type, 1 AS tier, pt.position
             FROM music_playlist_tracks pt
             JOIN music_tracks mt ON mt.id = pt.track_id
             JOIN media_objects mo ON mo.id = mt.cover_object_id
             WHERE pt.playlist_id = $1
         ) candidates
         ORDER BY tier, position
         LIMIT 1",
    )
    .bind(id)
    .fetch_optional(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    .ok_or(AuthError::ItemNotFound)?;

    let bytes = state.storage().get(&row.0).await.map_err(AuthError::Internal)?;
    Ok(([(header::CONTENT_TYPE, row.1)], Body::from(bytes)).into_response())
}

/// POST /api/v1/music/playlists/{id}/upload-cover — set the playlist's own cover. Owner only.
#[utoipa::path(post, path = "/playlists/{id}/upload-cover", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Playlist id")),
    request_body(content_type = "multipart/form-data", description = "fields: `cover` (the image file)"),
    responses((status = 201, description = "Stored"), (status = 400, body = crate::http::openapi::ErrorBody), (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn upload_playlist_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    db::music::find_playlist_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let mut upload: Option<TempUpload> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        if field.name().unwrap_or_default() == "cover" {
            upload = Some(save_field_to_temp(field).await?);
        }
    }

    let upload = upload.ok_or_else(|| AuthError::BadRequest("missing cover payload".to_string()))?;
    let ext = file_extension(&upload.file_name).unwrap_or("jpg");
    let object_key = crate::storage::family_key(
        family.family_id,
        format!("music/{}/playlist-covers/{}.{ext}", family.user_id, id),
    );

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;

    sqlx::query(
        "UPDATE music_playlists SET cover_object_id = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $1",
    )
    .bind(id)
    .bind(media_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(StatusCode::CREATED)
}

// ── Mapping helpers ───────────────────────────────────────────────────────

/// The album artist a compilation gets when its files carry the compilation flag but no
/// album-artist tag — the name every player uses for it.
const VARIOUS_ARTISTS: &str = "Various Artists";

fn track_to_response(t: models::MusicTrack, viewer_id: Uuid) -> TrackResponse {
    let album_artist = t.effective_album_artist();
    TrackResponse {
        id: t.id.to_string(),
        title: t.title,
        artist: t.artist,
        album: t.album,
        album_artist,
        genre: t.genre,
        track_number: t.track_number,
        disc_number: t.disc_number,
        disc_total: t.disc_total,
        duration_secs: t.duration_secs,
        cover_url: t
            .cover_object_id
            .map(|_| format!("/api/v1/music/tracks/{}/cover", t.id)),
        musicbrainz_recording_id: t.musicbrainz_recording_id,
        visibility: access::visibility_of(t.family_id).to_string(),
        is_owner: t.user_id == viewer_id,
        owner_id: t.user_id.to_string(),
        created_at: t.created_at.to_rfc3339(),
        updated_at: t.updated_at.to_rfc3339(),
        read_only: t.source.as_deref() == Some("folder"),
        source: t.source.unwrap_or_else(|| "upload".to_string()),
        size_bytes: t.size_bytes,
        sha256: t.sha256,
    }
}

/// How many tracks a playlist holds, and whether any of them brings artwork with it.
///
/// The second half is what makes `cover_url` honest: the cover endpoint falls back to a track's
/// artwork, so a playlist with no cover of its own still has one to serve.
async fn playlist_contents(state: &AppState, id: Uuid) -> Result<(i64, bool), AuthError> {
    sqlx::query_as::<_, (i64, bool)>(
        "SELECT COUNT(*), COALESCE(BOOL_OR(mt.cover_object_id IS NOT NULL), FALSE)
         FROM music_playlist_tracks pt
         LEFT JOIN music_tracks mt ON mt.id = pt.track_id
         WHERE pt.playlist_id = $1",
    )
    .bind(id)
    .fetch_one(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))
}

fn playlist_to_response(
    p: models::MusicPlaylist,
    track_count: i64,
    has_track_cover: bool,
    viewer_id: Uuid,
) -> PlaylistResponse {
    let has_cover = p.cover_object_id.is_some() || has_track_cover;
    PlaylistResponse {
        id: p.id.to_string(),
        name: p.name,
        description: p.description,
        cover_url: has_cover.then(|| format!("/api/v1/music/playlists/{}/cover", p.id)),
        track_count,
        visibility: access::visibility_of(p.family_id).to_string(),
        is_owner: p.user_id == viewer_id,
        created_at: p.created_at.to_rfc3339(),
        updated_at: p.updated_at.to_rfc3339(),
        generated_at: p.generated_at.map(|t| t.to_rfc3339()),
        kept_at: p.kept_at.map(|t| t.to_rfc3339()),
    }
}

fn trim_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn infer_title(file_name: &str) -> String {
    StdPath::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.replace(['_', '-'], " "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file_name.to_string())
}

fn sanitize(value: &str) -> String {
    value
        .replace(['\\', '/'], "-")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' })
        .collect::<String>()
}

fn file_extension(name: &str) -> Option<&str> {
    StdPath::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.trim_matches('.'))
        .filter(|e| !e.is_empty())
}

struct TempUpload {
    file_name: String,
    temp_file: NamedTempFile,
    content_type: String,
    size_bytes: i64,
}

async fn save_field_to_temp(mut field: axum::extract::multipart::Field<'_>) -> Result<TempUpload, AuthError> {
    let file_name = field
        .file_name()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "upload.bin".to_string());
    // "application/octet-stream" says nothing (the Mac sends it for AVIF); the name decides.
    let content_type = field
        .content_type()
        .filter(|t| *t != "application/octet-stream")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            MimeGuess::from_path(&file_name)
                .first_or_octet_stream()
                .essence_str()
                .to_string()
        });

    let temp_file = NamedTempFile::new().map_err(|e| AuthError::Internal(e.into()))?;
    let std_file = temp_file.reopen().map_err(|e| AuthError::Internal(e.into()))?;
    let mut writer = tokio::fs::File::from_std(std_file);
    let mut size_bytes = 0_i64;

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid upload chunk: {e}")))?
    {
        size_bytes += chunk.len() as i64;
        writer.write_all(&chunk).await.map_err(|e| AuthError::Internal(e.into()))?;
    }
    writer.flush().await.map_err(|e| AuthError::Internal(e.into()))?;

    Ok(TempUpload {
        file_name,
        temp_file,
        content_type,
        size_bytes,
    })
}

/// Materializes in-memory bytes (an embedded cover picture, in practice) as a `TempUpload` so
/// `store_temp_upload` can push it to storage the same way as a real multipart file field —
/// there's no reader to stream from here, just bytes already fully in hand.
fn write_temp_upload(bytes: &[u8], file_name: String, content_type: String) -> Result<TempUpload, AuthError> {
    use std::io::Write;
    let mut temp_file = NamedTempFile::new().map_err(|e| AuthError::Internal(e.into()))?;
    temp_file.write_all(bytes).map_err(|e| AuthError::Internal(e.into()))?;
    temp_file.flush().map_err(|e| AuthError::Internal(e.into()))?;
    Ok(TempUpload {
        file_name,
        temp_file,
        content_type,
        size_bytes: bytes.len() as i64,
    })
}

async fn store_temp_upload(
    storage: &crate::storage::ObjectStore,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object_key: &str,
    upload: &TempUpload,
) -> Result<Uuid, AuthError> {
    let bucket = storage.bucket().to_string();
    let path: PathBuf = upload.temp_file.path().to_path_buf();

    storage
        .put_path(object_key, &path, &upload.content_type)
        .await
        .map_err(AuthError::Internal)?;

    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO media_objects (bucket, object_key, content_type, size_bytes)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (bucket, object_key) DO UPDATE
             SET content_type = EXCLUDED.content_type,
                 size_bytes = EXCLUDED.size_bytes
         RETURNING id",
    )
    .bind(&bucket)
    .bind(object_key)
    .bind(&upload.content_type)
    .bind(upload.size_bytes)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))
}
