// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::users::models::{AuthIdentity, User};
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

/// Column list shared by every query below; keep in sync with `User`.
const USER_COLUMNS: &str = "id, email, display_name, role, is_active, avatar_object_id,
     recommendations_enabled, recommendations_changed_at, discovery_languages,
     email_verified_at, created_at, updated_at";

/// Records the proof; `true` the first time, `false` when it was already there.
pub async fn mark_email_verified(pool: &PgPool, user_id: Uuid) -> anyhow::Result<bool> {
    let rows = sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1 AND email_verified_at IS NULL")
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: mark email verified")?
        .rows_affected();
    Ok(rows > 0)
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<User>> {
    sqlx::query_as::<_, User>(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find user by id")
}

pub async fn find_by_email(pool: &PgPool, email: &str) -> anyhow::Result<Option<User>> {
    sqlx::query_as::<_, User>(&format!("SELECT {USER_COLUMNS} FROM users WHERE email = $1"))
        .bind(email)
        .fetch_optional(pool)
        .await
        .context("db: find user by email")
}

/// Find a user by email, ignoring case and surrounding whitespace.
///
/// Registration already lowercases and trims before storing, so this only
/// matters for what a *client* sends. It exists for the Subsonic surface,
/// where the username is typed into a phone: Android capitalises the first
/// letter of a text field by default, and `Kornel@example.com` missing the
/// row for `kornel@example.com` surfaces as error 40 "Wrong username or
/// password" — a wrong-credential message for a correct credential.
///
/// `lower(email)` rather than `email = lower($1)` because nothing guarantees
/// every historical row was normalized on the way in.
pub async fn find_by_email_ci(pool: &PgPool, email: &str) -> anyhow::Result<Option<User>> {
    sqlx::query_as::<_, User>(&format!(
        "SELECT {USER_COLUMNS} FROM users WHERE lower(email) = lower($1)"
    ))
    .bind(email.trim())
    .fetch_optional(pool)
    .await
    .context("db: find user by email (case-insensitive)")
}

pub async fn insert(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    role: &str,
) -> anyhow::Result<User> {
    let user = sqlx::query_as::<_, User>(&format!(
        "INSERT INTO users (email, display_name, role)
         VALUES ($1, $2, $3)
         RETURNING {USER_COLUMNS}",
    ))
    .bind(email)
    .bind(display_name)
    .bind(role)
    .fetch_one(pool)
    .await
    .context("db: insert user")?;

    // Every account gets its Subsonic key up front. Creating it lazily on the
    // first settings-page visit meant an account that had never opened that
    // page could not authenticate against `/rest` at all, and the protocol has
    // no error for "no key has been issued" — the client just saw error 40.
    crate::db::subsonic::get_or_create_key(pool, user.id).await?;

    Ok(user)
}

/// `(id, email)` for many users at once.
///
/// The Subsonic protocol identifies a playlist's owner by username, and
/// audio2's username is the account email. A shared playlist can belong to
/// another family member, so the listing needs more than the caller's own
/// name — and one lookup per playlist would be a query per row.
pub async fn emails_for(pool: &PgPool, ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, String)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, (Uuid, String)>("SELECT id, email FROM users WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: emails for users")
}

pub async fn list_all(pool: &PgPool) -> anyhow::Result<Vec<User>> {
    sqlx::query_as::<_, User>(&format!("SELECT {USER_COLUMNS} FROM users ORDER BY created_at"))
        .fetch_all(pool)
        .await
        .context("db: list all users")
}

/// One row per user with their family — for the instance-admin People tab.
/// A separate query rather than adding family columns to `User`/
/// `USER_COLUMNS`, which are shared by call sites that have no use for a
/// join (self-profile, `get_user`, etc.).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserWithFamily {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub family_id: Option<Uuid>,
    pub family_name: Option<String>,
}

pub async fn list_all_with_family(pool: &PgPool) -> anyhow::Result<Vec<UserWithFamily>> {
    sqlx::query_as::<_, UserWithFamily>(
        "SELECT u.id, u.email, u.display_name, u.role, u.is_active, u.created_at,
                fm.family_id, f.name AS family_name
         FROM users u
         LEFT JOIN family_members fm ON fm.user_id = u.id
         LEFT JOIN families f ON f.id = fm.family_id
         ORDER BY u.created_at",
    )
    .fetch_all(pool)
    .await
    .context("db: list all users with family")
}

/// New registrations by rolling window (last 1/7/30 days), as `(d1, d7,
/// d30)` — for the instance-admin dashboard. Excludes
/// provisioned-but-never-claimed family members (`db::families::list_members`'s
/// `pending` concept): they share `users.created_at`'s shape with a real
/// signup but were never issued credentials and may never be claimed, so
/// counting them would overstate real signups.
pub async fn registration_counts(pool: &PgPool) -> anyhow::Result<(i64, i64, i64)> {
    sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT
             COUNT(*) FILTER (WHERE created_at >= now() - interval '1 day')::BIGINT,
             COUNT(*) FILTER (WHERE created_at >= now() - interval '7 days')::BIGINT,
             COUNT(*) FILTER (WHERE created_at >= now() - interval '30 days')::BIGINT
         FROM users u
         WHERE EXISTS (SELECT 1 FROM auth_identities ai WHERE ai.user_id = u.id)",
    )
    .fetch_one(pool)
    .await
    .context("db: registration counts")
}

/// Sets or clears (`media_id: None`) the caller's own avatar. Generic over the executor (like
/// `db::media::upsert_object`) so the caller can run this in the same transaction as the
/// `media_objects` insert it points at — on a separate pool connection, the FK check would run
/// before that row is visible outside its own uncommitted transaction.
pub async fn set_avatar<'e, E>(executor: E, user_id: Uuid, media_id: Option<Uuid>) -> anyhow::Result<()>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query("UPDATE users SET avatar_object_id = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(user_id)
        .bind(media_id)
        .execute(executor)
        .await
        .context("db: set user avatar")?;
    Ok(())
}

/// Activates or deactivates a user. A deactivated user's existing sessions/tokens keep working
/// until separately revoked (the auth middleware doesn't re-check `is_active` per request — see
/// its own doc comment) and `POST /auth/login` refuses them — callers that want an immediate
/// cutoff, not just "can't log back in", must also revoke sessions/refresh tokens themselves
/// (`crate::families::block_member` does both).
pub async fn set_active(pool: &PgPool, user_id: Uuid, is_active: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET is_active = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(user_id)
        .bind(is_active)
        .execute(pool)
        .await
        .context("db: set user active")?;
    Ok(())
}

/// The avatar's storage key + content type, for streaming it back — `None` if the user has no
/// avatar or doesn't exist. Callers gate visibility (e.g. "same family") before calling this.
pub async fn find_avatar(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Option<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT mo.object_key, mo.content_type
         FROM media_objects mo
         JOIN users u ON u.avatar_object_id = mo.id
         WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find user avatar")
}

pub async fn find_identity_for_local(
    pool: &PgPool,
    user_id: Uuid,
) -> anyhow::Result<Option<AuthIdentity>> {
    sqlx::query_as::<_, AuthIdentity>(
        "SELECT id, user_id, provider, provider_subject, password_hash, created_at, updated_at
         FROM auth_identities
         WHERE user_id = $1 AND provider = 'local'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find local auth identity")
}

pub async fn find_identity_by_provider(
    pool: &PgPool,
    provider: &str,
    subject: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    sqlx::query_as::<_, AuthIdentity>(
        "SELECT id, user_id, provider, provider_subject, password_hash, created_at, updated_at
         FROM auth_identities
         WHERE provider = $1 AND provider_subject = $2",
    )
    .bind(provider)
    .bind(subject)
    .fetch_optional(pool)
    .await
    .context("db: find identity by provider subject")
}

pub async fn insert_local_identity(
    pool: &PgPool,
    user_id: Uuid,
    password_hash: &str,
) -> anyhow::Result<AuthIdentity> {
    sqlx::query_as::<_, AuthIdentity>(
        "INSERT INTO auth_identities (user_id, provider, password_hash)
         VALUES ($1, 'local', $2)
         RETURNING id, user_id, provider, provider_subject, password_hash, created_at, updated_at",
    )
    .bind(user_id)
    .bind(password_hash)
    .fetch_one(pool)
    .await
    .context("db: insert local identity")
}

pub async fn update_password(
    pool: &PgPool,
    user_id: Uuid,
    new_password_hash: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE auth_identities SET password_hash = $1, updated_at = CURRENT_TIMESTAMP
         WHERE user_id = $2 AND provider = 'local'",
    )
    .bind(new_password_hash)
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: update password")?;
    Ok(())
}

pub async fn delete_user(pool: &PgPool, user_id: Uuid) -> anyhow::Result<()> {
    // ON DELETE CASCADE handles auth_identities, sessions, and all user data
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete user")?;
    Ok(())
}

pub async fn update_display_name(
    pool: &PgPool,
    user_id: Uuid,
    display_name: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET display_name = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(display_name)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: update display name")?;
    Ok(())
}

/// Switch listening-derived recommendations on or off for one user.
///
/// Only ever called for the authenticated user themselves — there is no admin
/// path to this, on purpose (see migration 0061).
///
/// Nothing else happens here, and that is the design: there is no server-side
/// profile to build when it goes on, and none to delete when it goes off. If a
/// future phase ever adds one, deleting it belongs in this function, and the
/// switch stops being a preference and starts being consent.
/// The languages podcast discovery answers in. Stored folded and deduplicated
/// by the caller (`base_language`); an empty array means every language.
pub async fn set_discovery_languages(
    pool: &PgPool,
    user_id: Uuid,
    languages: &[String],
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE users
         SET discovery_languages = $1,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $2",
    )
    .bind(languages)
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: set discovery languages")?;
    Ok(())
}

pub async fn set_recommendations_enabled(
    pool: &PgPool,
    user_id: Uuid,
    enabled: bool,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE users
         SET recommendations_enabled = $1,
             recommendations_changed_at = CURRENT_TIMESTAMP,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $2",
    )
    .bind(enabled)
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: set recommendations enabled")?;
    Ok(())
}

pub async fn insert_oidc_identity(
    pool: &PgPool,
    user_id: Uuid,
    provider: &str,
    subject: &str,
) -> anyhow::Result<AuthIdentity> {
    sqlx::query_as::<_, AuthIdentity>(
        "INSERT INTO auth_identities (user_id, provider, provider_subject)
         VALUES ($1, $2, $3)
         RETURNING id, user_id, provider, provider_subject, password_hash, created_at, updated_at",
    )
    .bind(user_id)
    .bind(provider)
    .bind(subject)
    .fetch_one(pool)
    .await
    .context("db: insert oidc identity")
}
