// SPDX-License-Identifier: AGPL-3.0-or-later
//! From a Subsonic album or artist id to the entity behind it.
//!
//! The ids are the entities' own (migration 0100): an album id is an
//! `albums` row, an artist id a `contributors` row. Either answers only when
//! the viewer can see at least one track behind it.

use crate::db::{self, access::Viewer, subsonic::AlbumRow};
use sqlx::PgPool;
use uuid::Uuid;

pub enum Named {
    Artist { id: Uuid, name: String },
    Album(AlbumRow),
}

/// `None` when the id is not an album or artist this viewer can see.
pub async fn resolve(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<Named>> {
    if let Some(album) = db::subsonic::find_album(pool, viewer, id).await? {
        return Ok(Some(Named::Album(album)));
    }
    Ok(db::subsonic::find_artist(pool, viewer, id).await?.map(|name| Named::Artist { id, name }))
}
