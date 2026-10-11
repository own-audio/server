// SPDX-License-Identifier: AGPL-3.0-or-later
//! Music entities (migration 0100; docs/data-model-plan.md). They are kept in
//! step by triggers; this module only clears what nothing points at any more.
//! Book credits (migration 0102) count like any other.
use anyhow::Context;
use sqlx::PgPool;

/// Removes albums, release groups, recordings, genres and contributors that no
/// track or credit refers to — left behind by a retag, a deletion or a user
/// moving household. Returns how many rows went.
pub async fn sweep_orphans(pool: &PgPool) -> anyhow::Result<u64> {
    let mut gone = 0;
    for sql in [
        "DELETE FROM albums a WHERE NOT EXISTS (SELECT 1 FROM music_tracks_all t WHERE t.album_id = a.id)",
        "DELETE FROM release_groups rg WHERE NOT EXISTS (SELECT 1 FROM albums a WHERE a.release_group_id = rg.id)",
        "DELETE FROM recordings r WHERE NOT EXISTS (SELECT 1 FROM music_tracks_all t WHERE t.recording_id = r.id)",
        "DELETE FROM genres g WHERE NOT EXISTS (SELECT 1 FROM track_genres tg WHERE tg.genre_id = g.id)",
        // Album credits go with their album; release-group and recording credits are not written yet.
        "DELETE FROM credits c WHERE c.item_kind = 'album' AND NOT EXISTS (SELECT 1 FROM albums a WHERE a.id = c.item_id)",
        // One made or corrected by hand stays, credited or not (an author created
        // before any book is linked to them).
        "DELETE FROM contributors k WHERE k.edited_at IS NULL AND NOT EXISTS (SELECT 1 FROM credits c WHERE c.contributor_id = k.id)",
    ] {
        gone += sqlx::query(sql).execute(pool).await.context("db: sweep orphaned music entities")?.rows_affected();
    }
    Ok(gone)
}
