// SPDX-License-Identifier: AGPL-3.0-or-later
//! Personal access tokens (migration 0105). The secret is shown once; only its
//! SHA-256 is kept.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// What every token starts with, so a leaked one is recognisable (and a
/// secret scanner can be told about it).
pub const PREFIX: &str = "oat_";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PersonalToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct Authenticated {
    #[sqlx(flatten)]
    token: PersonalToken,
    role: String,
}

const COLS: &str = "id, user_id, name, scopes, created_at, last_used_at, expires_at";

pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    token_hash: &str,
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
) -> anyhow::Result<PersonalToken> {
    sqlx::query_as(&format!(
        "INSERT INTO personal_tokens (user_id, name, token_hash, scopes, expires_at) VALUES ($1, $2, $3, $4, $5)
         RETURNING {COLS}"
    ))
    .bind(user_id)
    .bind(name)
    .bind(token_hash)
    .bind(scopes)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .context("db: create personal token")
}

/// The live tokens of an account, newest first.
pub async fn list(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<PersonalToken>> {
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM personal_tokens
          WHERE user_id = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())
          ORDER BY created_at DESC, id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list personal tokens")
}

/// A token that may be used now: not revoked, not expired, its account active.
/// Marks it used, at most once a minute.
pub async fn authenticate(pool: &PgPool, token_hash: &str) -> anyhow::Result<Option<(PersonalToken, String)>> {
    let row: Option<Authenticated> = sqlx::query_as(
        "SELECT t.id, t.user_id, t.name, t.scopes, t.created_at, t.last_used_at, t.expires_at, u.role
           FROM personal_tokens t JOIN users u ON u.id = t.user_id
          WHERE t.token_hash = $1 AND t.revoked_at IS NULL
            AND (t.expires_at IS NULL OR t.expires_at > now()) AND u.is_active",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .context("db: authenticate personal token")?;
    let Some(Authenticated { token, role }) = row else {
        return Ok(None);
    };
    if token.last_used_at.is_none_or(|t| Utc::now() - t > chrono::Duration::minutes(1)) {
        sqlx::query("UPDATE personal_tokens SET last_used_at = now() WHERE id = $1")
            .bind(token.id)
            .execute(pool)
            .await
            .context("db: personal token used")?;
    }
    Ok(Some((token, role)))
}

pub async fn revoke(pool: &PgPool, user_id: Uuid, id: Uuid) -> anyhow::Result<bool> {
    Ok(sqlx::query("UPDATE personal_tokens SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: revoke personal token")?
        .rows_affected()
        == 1)
}

/// RFC 7009: the token itself, whoever presents it.
pub async fn revoke_by_hash(pool: &PgPool, token_hash: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE personal_tokens SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL")
        .bind(token_hash)
        .execute(pool)
        .await
        .context("db: revoke personal token by hash")?;
    Ok(())
}
