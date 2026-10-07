// SPDX-License-Identifier: AGPL-3.0-or-later
/// Favourites, ratings and bookmarks.
///
/// Songs are addressed by their real row id. Artists and albums arrive as the
/// synthetic UUID v5 values `subsonic::ids` derives, which have no rows behind
/// them, so they are resolved back to the artist/album names they were built
/// from and stored by name — see `migrations/0036`.
use crate::app::AppState;
use crate::db;
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::extract::SubsonicQuery;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
use crate::subsonic::resolve;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

fn ok(auth: &SubsonicAuthUser, body: Value) -> Response {
    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), body)
}

fn err(auth: &SubsonicAuthUser, code: SubsonicErrorCode) -> Response {
    envelope::error(auth.format, auth.jsonp_callback.as_deref(), code, None)
}

/// All three id parameters are repeatable in the protocol, so a client may
/// star a whole selection in one call.
#[derive(Deserialize)]
pub struct StarParams {
    #[serde(default)]
    id: Vec<String>,
    #[serde(default)]
    #[serde(rename = "albumId")]
    album_id: Vec<String>,
    #[serde(default)]
    #[serde(rename = "artistId")]
    artist_id: Vec<String>,
}

pub async fn star(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<StarParams>,
) -> Response {
    apply_stars(&auth, &state, &params, true).await
}

pub async fn unstar(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<StarParams>,
) -> Response {
    apply_stars(&auth, &state, &params, false).await
}

async fn apply_stars(
    auth: &SubsonicAuthUser,
    state: &AppState,
    params: &StarParams,
    starred: bool,
) -> Response {
    // At least one of the three id kinds is required. Answering "ok" to a call
    // that named nothing reported success for a write that never happened, so
    // a client whose id parameter went missing saw its star stick in the UI
    // and then vanish on the next refresh.
    if params.id.is_empty() && params.album_id.is_empty() && params.artist_id.is_empty() {
        return err(auth, SubsonicErrorCode::MissingParam);
    }

    for raw in &params.id {
        let Ok(track_id) = raw.parse::<Uuid>() else { continue };
        // Visibility is checked before writing: a starred id the viewer cannot
        // see would otherwise let them probe for another member's track ids.
        match db::music::find_track(state.db(), track_id, auth.viewer()).await {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(_) => return err(auth, SubsonicErrorCode::Generic),
        }
        let result = if starred {
            db::music::star_track(state.db(), auth.user_id, track_id).await
        } else {
            db::music::unstar_track(state.db(), auth.user_id, track_id).await
        };
        if result.is_err() {
            return err(auth, SubsonicErrorCode::Generic);
        }
    }

    if !params.album_id.is_empty() {
        for raw in &params.album_id {
            let Ok(target) = raw.parse::<Uuid>() else { continue };
            let (artist, album) = match resolve::resolve(state.db(), auth.viewer(), target).await {
                Ok(Some(resolve::Named::Album { artist, album })) => (artist, album),
                Ok(_) => continue,
                Err(_) => return err(auth, SubsonicErrorCode::Generic),
            };
            let result = if starred {
                db::music::star_group(state.db(), auth.user_id, "album", &artist, &album).await
            } else {
                db::music::unstar_group(state.db(), auth.user_id, "album", &artist, &album).await
            };
            if result.is_err() {
                return err(auth, SubsonicErrorCode::Generic);
            }
        }
    }

    if !params.artist_id.is_empty() {
        for raw in &params.artist_id {
            let Ok(target) = raw.parse::<Uuid>() else { continue };
            let artist = match resolve::resolve(state.db(), auth.viewer(), target).await {
                Ok(Some(resolve::Named::Artist(artist))) => artist,
                Ok(_) => continue,
                Err(_) => return err(auth, SubsonicErrorCode::Generic),
            };
            let result = if starred {
                db::music::star_group(state.db(), auth.user_id, "artist", &artist, "").await
            } else {
                db::music::unstar_group(state.db(), auth.user_id, "artist", &artist, "").await
            };
            if result.is_err() {
                return err(auth, SubsonicErrorCode::Generic);
            }
        }
    }

    ok(auth, json!({}))
}

#[derive(Deserialize)]
pub struct SetRatingParams {
    id: String,
    rating: Option<i16>,
}

/// `id` may be a song, an album or an artist.
///
/// The protocol allows all three and real clients use them: Amperfy has an
/// "Album Rating Sync" that posts an album id here. Rating songs only meant
/// that arrived as error 70 — the id resolved to no track — and the client
/// surfaced it as a sync failure.
pub async fn set_rating(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<SetRatingParams>,
) -> Response {
    let Ok(target) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };
    let Some(rating) = params.rating else {
        return err(&auth, SubsonicErrorCode::MissingParam);
    };

    // A song id is a real row, so try it first and cheapest.
    match db::music::find_track(state.db(), target, auth.viewer()).await {
        Ok(Some(_)) => {
            return match db::music::set_track_rating(state.db(), auth.user_id, target, rating).await {
                Ok(()) => ok(&auth, json!({})),
                Err(_) => err(&auth, SubsonicErrorCode::Generic),
            };
        }
        Ok(None) => {}
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    }

    match resolve_group(&auth, &state, target).await {
        Ok(Some((kind, artist, album))) => {
            match db::music::set_group_rating(state.db(), auth.user_id, kind, &artist, &album, rating)
                .await
            {
                Ok(()) => ok(&auth, json!({})),
                Err(_) => err(&auth, SubsonicErrorCode::Generic),
            }
        }
        Ok(None) => err(&auth, SubsonicErrorCode::NotFound),
        Err(()) => err(&auth, SubsonicErrorCode::Generic),
    }
}

/// Maps a synthetic album or artist id back to the names it was derived from.
///
/// Returns `("album", artist, album)` or `("artist", artist, "")`, matching the
/// shape the group tables store.
async fn resolve_group(
    auth: &SubsonicAuthUser,
    state: &AppState,
    target: Uuid,
) -> Result<Option<(&'static str, String, String)>, ()> {
    Ok(match resolve::resolve(state.db(), auth.viewer(), target).await.map_err(|_| ())? {
        Some(resolve::Named::Album { artist, album }) => Some(("album", artist, album)),
        Some(resolve::Named::Artist(artist)) => Some(("artist", artist, String::new())),
        None => None,
    })
}

// ── Bookmarks ─────────────────────────────────────────────────────────────

pub async fn get_bookmarks(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    let marks = match db::music::list_music_bookmarks(state.db(), auth.user_id).await {
        Ok(m) => m,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let mut tracks = Vec::with_capacity(marks.len());
    for mark in &marks {
        // A bookmark can outlive the viewer's access to the track (a family
        // share withdrawn, say), so each one is re-checked rather than
        // trusted because it is in their own row.
        if let Ok(Some(track)) = db::music::find_track(state.db(), mark.track_id, auth.viewer()).await {
            tracks.push((mark, track));
        }
    }

    let only_tracks: Vec<_> = tracks.iter().map(|(_, t)| t.clone()).collect();
    let ctx = crate::subsonic::browsing::SongContext::load(state.db(), auth.user_id, &only_tracks).await;

    let entries: Vec<Value> = tracks
        .iter()
        .map(|(mark, track)| {
            json!({
                "position": (mark.position_secs * 1000.0) as i64,
                "username": auth.username,
                "comment": mark.label,
                "created": mark.created_at.to_rfc3339(),
                "changed": mark.created_at.to_rfc3339(),
                "entry": crate::subsonic::browsing::song_json(track, &ctx),
            })
        })
        .collect();

    ok(&auth, json!({ "bookmarks": { "bookmark": entries } }))
}

#[derive(Deserialize)]
pub struct CreateBookmarkParams {
    id: String,
    /// Milliseconds, per the Subsonic spec.
    position: i64,
    comment: Option<String>,
}

pub async fn create_bookmark(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<CreateBookmarkParams>,
) -> Response {
    let Ok(track_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    match db::music::find_track(state.db(), track_id, auth.viewer()).await {
        Ok(Some(_)) => {}
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    }

    if db::music::upsert_music_bookmark(
        state.db(),
        auth.user_id,
        track_id,
        params.position.max(0) as f64 / 1000.0,
        params.comment.as_deref(),
    )
    .await
    .is_err()
    {
        return err(&auth, SubsonicErrorCode::Generic);
    }
    ok(&auth, json!({}))
}

#[derive(Deserialize)]
pub struct DeleteBookmarkParams {
    id: String,
}

pub async fn delete_bookmark(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<DeleteBookmarkParams>,
) -> Response {
    let Ok(track_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };
    if db::music::delete_music_bookmark(state.db(), auth.user_id, track_id)
        .await
        .is_err()
    {
        return err(&auth, SubsonicErrorCode::Generic);
    }
    ok(&auth, json!({}))
}

#[cfg(test)]
mod tests {
    use crate::subsonic::ids;
    use uuid::Uuid;

    /// The regression behind Amperfy's "Album Rating Sync" failure: an album id
    /// is a synthetic UUID v5, not a `music_tracks` row, so a `setRating` that
    /// only ever looked for a track answered error 70 for every album.
    #[test]
    fn an_album_id_is_not_a_track_id() {
        let user = Uuid::new_v4();
        let album = ids::album_id(user, "Toto", "Toto IV");
        let artist = ids::artist_id(user, "Toto");
        assert_ne!(album, artist);
        // Stable across calls, which is what lets a rating stored by name be
        // found again by the id a client sends back.
        assert_eq!(album, ids::album_id(user, "Toto", "Toto IV"));
    }

    /// Group keys are normalized, so the id a client echoes back resolves to
    /// the same row whatever case the tags happen to use.
    #[test]
    fn group_ids_ignore_case_and_padding() {
        let user = Uuid::new_v4();
        assert_eq!(
            ids::album_id(user, "Toto", "Toto IV"),
            ids::album_id(user, " toto ", " TOTO IV ")
        );
    }
}
