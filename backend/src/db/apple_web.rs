// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign in with Apple in a browser (migration 0135, `auth::apple_web`): a row from the start
//! of a sign-in until its code is spent. Each step moves it on in one statement, so a state or
//! a code is good once however many requests race for it.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// SHA-256 hex of a state or code: the only form stored.
pub fn hash(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

pub struct NewSignIn<'a> {
    pub state_hash: &'a str,
    pub nonce: &'a str,
    pub code_challenge: &'a str,
    pub redirect_uri: &'a str,
    pub app_state: Option<&'a str>,
    pub expires_at: DateTime<Utc>,
}

/// Records a started sign-in, and sweeps the rows of sign-ins long over.
pub async fn create(pool: &PgPool, new: &NewSignIn<'_>) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM apple_web_sign_ins WHERE expires_at < now() - interval '1 hour'")
        .execute(pool)
        .await
        .context("sweep old apple web sign-ins")?;
    sqlx::query(
        "INSERT INTO apple_web_sign_ins (state_hash, nonce, code_challenge, redirect_uri, app_state, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new.state_hash)
    .bind(new.nonce)
    .bind(new.code_challenge)
    .bind(new.redirect_uri)
    .bind(new.app_state)
    .bind(new.expires_at)
    .execute(pool)
    .await
    .context("insert apple web sign-in")?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct Returning {
    pub id: Uuid,
    pub nonce: String,
    pub redirect_uri: String,
    pub app_state: Option<String>,
}

/// Apple's answer for a live, started sign-in; the state is spent on the way out.
pub async fn take_state(pool: &PgPool, state_hash: &str) -> anyhow::Result<Option<Returning>> {
    sqlx::query_as::<_, Returning>(
        "UPDATE apple_web_sign_ins SET status = 'returned'
         WHERE state_hash = $1 AND status = 'started' AND expires_at > now()
         RETURNING id, nonce, redirect_uri, app_state",
    )
    .bind(state_hash)
    .fetch_optional(pool)
    .await
    .context("take apple web sign-in state")
}

pub struct Signed<'a> {
    pub code_hash: &'a str,
    pub subject: &'a str,
    pub email: &'a str,
    pub email_verified: bool,
    pub full_name: Option<&'a str>,
    pub expires_at: DateTime<Utc>,
}

/// Who Apple says signed in, and the hash of the code the app will spend for it.
pub async fn sign(pool: &PgPool, id: Uuid, signed: &Signed<'_>) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE apple_web_sign_ins
         SET status = 'signed', code_hash = $2, subject = $3, email = $4, email_verified = $5,
             full_name = $6, expires_at = $7
         WHERE id = $1 AND status = 'returned'",
    )
    .bind(id)
    .bind(signed.code_hash)
    .bind(signed.subject)
    .bind(signed.email)
    .bind(signed.email_verified)
    .bind(signed.full_name)
    .bind(signed.expires_at)
    .execute(pool)
    .await
    .context("sign apple web sign-in")?;
    Ok(())
}

/// A sign-in that ended without one (cancelled, or a token that did not verify).
pub async fn delete(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM apple_web_sign_ins WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("delete apple web sign-in")?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct Taken {
    pub code_challenge: String,
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub full_name: Option<String>,
}

/// Spends a live code: the row is gone after this, whatever the verifier turns out to be.
pub async fn take_code(pool: &PgPool, code_hash: &str) -> anyhow::Result<Option<Taken>> {
    sqlx::query_as::<_, Taken>(
        "DELETE FROM apple_web_sign_ins
         WHERE code_hash = $1 AND status = 'signed' AND expires_at > now()
           AND subject IS NOT NULL AND email IS NOT NULL
         RETURNING code_challenge, subject, email, email_verified, full_name",
    )
    .bind(code_hash)
    .fetch_optional(pool)
    .await
    .context("take apple web sign-in code")
}
