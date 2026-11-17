// SPDX-License-Identifier: AGPL-3.0-or-later
//! The disc-number back-fill (docs/api-v2-completion-plan.md, phase 12): tracks uploaded before
//! the server kept disc numbers (migration 0083) have their file read once for it. v1 left this
//! to a client calling `POST /music/tracks/discs` until nothing was left; v2 has no such route,
//! and the worker does it on its own, a few files a pass like the checksum sweep, while the jobs
//! list shows how many are left.

use crate::db;
use crate::storage::ObjectStore;
use sqlx::PgPool;

pub const JOB_TYPE: &str = "disc_backfill";

/// Files read per job. Each is a whole download on object storage; the next job follows as
/// soon as this one is done, so a small batch costs little time and keeps the queue fair.
pub const BATCH: i64 = 20;

/// Reads up to [`BATCH`] files and answers `{checked, left}` for the jobs list. A file that
/// cannot be read counts as read: it has nothing to add, and would otherwise come back forever.
pub async fn execute(pool: &PgPool, storage: &ObjectStore) -> anyhow::Result<serde_json::Value> {
    let batch = db::admin::tracks_without_disc(pool, BATCH).await?;
    for (id, key) in &batch {
        let tags = match storage.download_to_temp(key).await {
            Ok(temp) => tokio::task::spawn_blocking(move || super::read_embedded_tags(temp.path())).await?,
            Err(e) => {
                tracing::warn!(track = %id, "disc number unreadable: {e:#}");
                super::EmbeddedTags::default()
            }
        };
        db::music::set_disc(pool, *id, tags.disc_number, tags.disc_total).await?;
    }
    let left = db::admin::count_without_disc(pool).await?;
    Ok(serde_json::json!({ "checked": batch.len(), "left": left }))
}
