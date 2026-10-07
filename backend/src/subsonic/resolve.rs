// SPDX-License-Identifier: AGPL-3.0-or-later
//! From a Subsonic album or artist id back to the names it was made from.
//!
//! The ids are name-based UUIDs (`ids.rs`), which no index can find, so a
//! lookup used to aggregate the whole catalog and recompute every id. Now the
//! names behind each id are kept in `subsonic_ids`: a lookup is a key read,
//! and only an id the table does not know yet (a new album, the first request
//! after an upgrade) costs the old full pass, which also records every id it
//! computes on the way.

use super::ids;
use crate::db::{self, access::Viewer};
use sqlx::PgPool;
use uuid::Uuid;

pub enum Named {
    Artist(String),
    Album { artist: String, album: String },
}

/// One full pass at a time: a client opening an album grid asks for fifty
/// covers at once, and fifty identical passes would take every connection.
static FILL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `None` when the id is not an album or artist this viewer can see.
pub async fn resolve(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<Named>> {
    if let Some(named) = cached(pool, viewer, id).await? {
        return Ok(Some(named));
    }
    let _guard = FILL.lock().await;
    // Someone else's pass may have recorded it while this one waited.
    if let Some(named) = cached(pool, viewer, id).await? {
        return Ok(Some(named));
    }
    fill(pool, viewer).await?;
    cached(pool, viewer, id).await
}

/// A remembered id, if its album or artist is still visible to the viewer.
async fn cached(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<Named>> {
    let Some((artist, album)) = db::subsonic::cached_names(pool, viewer.user_id, id).await? else {
        return Ok(None);
    };
    Ok(match album {
        Some(album) => db::subsonic::find_album(pool, viewer, &artist, &album)
            .await?
            .map(|row| Named::Album { artist: row.artist, album: row.album }),
        None => (!db::subsonic::list_artist_albums(pool, viewer, &artist).await?.is_empty()).then_some(Named::Artist(artist)),
    })
}

/// Record the id of every album and artist the viewer can see.
async fn fill(pool: &PgPool, viewer: Viewer) -> anyhow::Result<()> {
    let albums = db::subsonic::list_distinct_albums(pool, viewer).await?;
    let mut seen_artists = std::collections::HashSet::new();
    let (mut id_list, mut artists, mut album_names) = (Vec::new(), Vec::new(), Vec::new());
    for row in &albums {
        id_list.push(ids::album_id(viewer.user_id, &row.artist, &row.album));
        artists.push(row.artist.clone());
        album_names.push(Some(row.album.clone()));
        if seen_artists.insert(row.artist.as_str()) {
            id_list.push(ids::artist_id(viewer.user_id, &row.artist));
            artists.push(row.artist.clone());
            album_names.push(None);
        }
    }
    for start in (0..id_list.len()).step_by(10_000) {
        let end = (start + 10_000).min(id_list.len());
        db::subsonic::remember_names(pool, viewer.user_id, &id_list[start..end], &artists[start..end], &album_names[start..end])
            .await?;
    }
    Ok(())
}
