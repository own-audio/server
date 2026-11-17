// SPDX-License-Identifier: AGPL-3.0-or-later
//! Companion files as `/api/v2` reads them (completion plan phase 11): the images,
//! booklets, lyrics and cue sheets of the own.audio folder (migration 0079), kind `file`.
//!
//! A file is seen through the member policy of the folder it sits in (`media_kind`), and
//! carries no grants of its own: `audio2_can_access` decides, as v1's sync tree does.
use crate::db::access::Viewer;
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FileRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub object_id: Uuid,
    pub path: Option<String>,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
    pub sha256: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const FILE_COLS: &str = "t.id, t.user_id, t.family_id, t.object_id,
    (SELECT p.path FROM sync_paths p WHERE p.kind = 'companion_file' AND p.item_id = t.id) AS path,
    m.size_bytes, m.content_type, m.sha256, t.created_at, t.updated_at";

/// Bound as `$1` user, `$2` family, `$3` family admin, as `db::catalog` binds the viewer.
pub(crate) const VISIBLE: &str = "audio2_can_access($1, $2, $3, t.media_kind, t.id, t.user_id, t.family_id)";

pub async fn files_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<FileRow>> {
    let sql = format!(
        "SELECT {FILE_COLS} FROM companion_files t JOIN media_objects m ON m.id = t.object_id
          WHERE t.id = ANY($4) AND {VISIBLE} ORDER BY t.id"
    );
    sqlx::query_as::<_, FileRow>(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: files by ids")
}

pub async fn find_file(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<FileRow>> {
    Ok(files_by_ids(pool, viewer, &[id]).await?.pop())
}

/// The caller's own live file: its object and its path.
pub async fn own_file(pool: &PgPool, owner: Uuid, id: Uuid) -> anyhow::Result<Option<(Uuid, String)>> {
    sqlx::query_as(
        "SELECT t.object_id, p.path FROM companion_files t
           JOIN sync_paths p ON p.kind = 'companion_file' AND p.item_id = t.id
          WHERE t.id = $1 AND t.user_id = $2",
    )
    .bind(id)
    .bind(owner)
    .fetch_optional(pool)
    .await
    .context("db: own companion file")
}

/// Share the owner's file with the household (`Some(family)`) or keep it private. False when
/// it is not the owner's live file.
pub async fn set_visibility(pool: &PgPool, owner: Uuid, id: Uuid, family_id: Option<Uuid>) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE companion_files SET family_id = $3, updated_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(owner)
        .bind(family_id)
        .execute(pool)
        .await
        .context("db: companion file visibility")?;
    Ok(done.rows_affected() > 0)
}

/// The folder an item of the caller's sits in, for a file put next to it: a book's folder, a
/// track's directory, or the directory of the caller's first track of an album.
pub async fn folder_of(pool: &PgPool, owner: Uuid, kind: &str, id: Uuid) -> anyhow::Result<Option<String>> {
    let sql = match kind {
        "book" => "SELECT p.path FROM sync_live_paths p WHERE p.kind = 'audiobook' AND p.item_id = $1 AND p.user_id = $2",
        "track" => "SELECT regexp_replace(p.path, '/[^/]*$', '') FROM sync_live_paths p
                     WHERE p.kind = 'music_track' AND p.item_id = $1 AND p.user_id = $2",
        "album" => "SELECT regexp_replace(p.path, '/[^/]*$', '') FROM music_tracks t
                      JOIN sync_live_paths p ON p.kind = 'music_track' AND p.item_id = t.id
                     WHERE t.album_id = $1 AND t.user_id = $2
                     ORDER BY COALESCE(t.disc_number, 1), COALESCE(t.track_number, 0), t.id LIMIT 1",
        _ => return Ok(None),
    };
    sqlx::query_scalar(sql)
        .bind(id)
        .bind(owner)
        .fetch_optional(pool)
        .await
        .context("db: folder of an item")
}
