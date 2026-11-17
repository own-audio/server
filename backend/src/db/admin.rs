// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the server's administrator reads on `/api/v2/admin` (docs/api-v2-completion-plan.md,
//! phase 12): accounts, households and jobs a page at a time, the server's totals, and the
//! disc-number back-fill's queue. v1's admin routes keep their own queries.

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub family_id: Option<Uuid>,
    pub family_name: Option<String>,
    pub family_role: Option<String>,
    /// Made by a household admin and not claimed yet: no way to sign in.
    pub pending: bool,
}

const USER_SELECT: &str = "SELECT u.id, u.email, u.display_name, u.role, u.is_active, u.email_verified_at, u.created_at,
            fm.family_id, f.name AS family_name, fm.role AS family_role,
            NOT EXISTS (SELECT 1 FROM auth_identities ai WHERE ai.user_id = u.id) AS pending
       FROM users u
       LEFT JOIN family_members fm ON fm.user_id = u.id
       LEFT JOIN families f ON f.id = fm.family_id";

/// Accounts in the order they were made; `q` matches a part of the address or the name.
pub async fn users_page(pool: &PgPool, q: Option<&str>, after: Option<(DateTime<Utc>, Uuid)>, limit: i64) -> anyhow::Result<Vec<UserRow>> {
    sqlx::query_as::<_, UserRow>(&format!(
        "{USER_SELECT}
          WHERE ($1::text IS NULL OR position(lower($1) IN lower(u.email)) > 0 OR position(lower($1) IN lower(u.display_name)) > 0)
            AND ($2::timestamptz IS NULL OR (u.created_at, u.id) > ($2, $3))
          ORDER BY u.created_at, u.id
          LIMIT $4"
    ))
    .bind(q)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: admin users page")
}

pub async fn user(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<UserRow>> {
    sqlx::query_as::<_, UserRow>(&format!("{USER_SELECT} WHERE u.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: admin user")
}

/// Administrators of the server who may still sign in.
pub async fn active_instance_admins(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'admin' AND is_active")
        .fetch_one(pool)
        .await
        .context("db: count instance admins")
}

pub async fn set_role(pool: &PgPool, id: Uuid, role: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET role = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(id)
        .bind(role)
        .execute(pool)
        .await
        .context("db: set instance role")?;
    Ok(())
}

#[derive(sqlx::FromRow)]
pub struct FamilyRow {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub member_count: i64,
    /// Members who may sign in; none, with members, is a locked household.
    pub active_members: i64,
}

/// Which households a list keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilyState {
    /// Someone in it may sign in, or nobody is in it.
    Active,
    /// It has members and none of them may sign in.
    Locked,
    /// Something of it is in the trash.
    Trashed,
}

const FAMILY_SELECT: &str = "SELECT f.id, f.name, f.created_at,
            (SELECT count(*) FROM family_members m WHERE m.family_id = f.id) AS member_count,
            (SELECT count(*) FROM family_members m JOIN users u ON u.id = m.user_id
              WHERE m.family_id = f.id AND u.is_active) AS active_members
       FROM families f";

/// Households in the order they were made, by state.
pub async fn families_page(
    pool: &PgPool,
    state: Option<FamilyState>,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<FamilyRow>> {
    let filter = match state {
        None => "TRUE".to_string(),
        Some(FamilyState::Active) => "NOT (member_count > 0 AND active_members = 0)".to_string(),
        Some(FamilyState::Locked) => "member_count > 0 AND active_members = 0".to_string(),
        Some(FamilyState::Trashed) => format!(
            "EXISTS (SELECT 1 FROM ({}) x JOIN family_members fm ON fm.user_id = x.owner_id WHERE fm.family_id = h.id)",
            super::trash::trashed_union()
        ),
    };
    sqlx::query_as::<_, FamilyRow>(&format!(
        "SELECT h.* FROM ({FAMILY_SELECT}) h
          WHERE {filter}
            AND ($1::timestamptz IS NULL OR (h.created_at, h.id) > ($1, $2))
          ORDER BY h.created_at, h.id
          LIMIT $3"
    ))
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: admin families page")
}

pub async fn family(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<FamilyRow>> {
    sqlx::query_as::<_, FamilyRow>(&format!("{FAMILY_SELECT} WHERE f.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: admin family")
}

/// The server's totals: accounts and households.
pub async fn totals(pool: &PgPool) -> anyhow::Result<(i64, i64)> {
    sqlx::query_as("SELECT (SELECT count(*) FROM users), (SELECT count(*) FROM families)")
        .fetch_one(pool)
        .await
        .context("db: admin totals")
}

/// Jobs newest first, of one kind or state when asked.
pub async fn jobs_page(
    pool: &PgPool,
    kind: Option<&str>,
    state: Option<&str>,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<crate::jobs::models::Job>> {
    sqlx::query_as::<_, crate::jobs::models::Job>(
        "SELECT id, job_type, status, payload, result, error, attempts, max_attempts, scheduled_at, started_at, completed_at, created_at
           FROM jobs
          WHERE ($1::text IS NULL OR job_type = $1)
            AND ($2::text IS NULL OR status = $2)
            AND ($3::timestamptz IS NULL OR (created_at, id) < ($3, $4))
          ORDER BY created_at DESC, id DESC
          LIMIT $5",
    )
    .bind(kind)
    .bind(state)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: admin jobs page")
}

#[derive(sqlx::FromRow)]
pub struct FolderRow {
    pub id: Uuid,
    pub path: String,
    pub kind: String,
    pub visibility: String,
    pub scan_started_at: Option<DateTime<Utc>>,
    pub scan_finished_at: Option<DateTime<Utc>>,
    pub scan_error: Option<String>,
    pub files_seen: i32,
    pub files_added: i32,
    pub files_missing: i64,
}

/// The household's library folders and how their last scan went (v1's query).
pub async fn library_folders(pool: &PgPool, family_id: Uuid) -> anyhow::Result<Vec<FolderRow>> {
    sqlx::query_as::<_, FolderRow>(
        "SELECT f.id, f.path, f.kind, f.visibility, f.scan_started_at, f.scan_finished_at, f.scan_error,
                f.files_seen, f.files_added,
                (SELECT count(*) FROM library_files lf WHERE lf.folder_id = f.id AND lf.missing) AS files_missing
           FROM library_folders f
          WHERE f.family_id = $1
          ORDER BY f.path",
    )
    .bind(family_id)
    .fetch_all(pool)
    .await
    .context("db: library folders")
}

/// Tracks of anyone whose file was never read for a disc number, oldest first, with the key
/// of the file to read.
pub async fn tracks_without_disc(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<(Uuid, String)>> {
    sqlx::query_as(
        "SELECT t.id, mo.object_key FROM music_tracks_all t JOIN media_objects mo ON mo.id = t.audio_object_id
          WHERE NOT t.disc_checked AND t.trashed_at IS NULL
          ORDER BY t.created_at, t.id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: tracks without disc")
}

pub async fn count_without_disc(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM music_tracks_all WHERE NOT disc_checked AND trashed_at IS NULL")
        .fetch_one(pool)
        .await
        .context("db: count tracks without disc")
}

/// One back-fill pass at a time: a pass waiting or running covers this one.
pub async fn enqueue_disc_backfill(pool: &PgPool) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "INSERT INTO jobs (job_type, payload, scheduled_at)
         SELECT $1, '{}'::jsonb, CURRENT_TIMESTAMP
          WHERE EXISTS (SELECT 1 FROM music_tracks_all WHERE NOT disc_checked AND trashed_at IS NULL)
            AND NOT EXISTS (SELECT 1 FROM jobs WHERE job_type = $1 AND status IN ('pending', 'running'))",
    )
    .bind(crate::music::discs::JOB_TYPE)
    .execute(pool)
    .await
    .context("db: enqueue disc back-fill")?;
    Ok(result.rows_affected() > 0)
}
