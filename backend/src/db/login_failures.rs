// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wrong passwords in a row, per typed email (migration 0095).
use anyhow::Context;
use sqlx::PgPool;
use std::time::Duration;

/// Seconds until the email may try again, if it is locked right now.
pub async fn locked_for_secs(pool: &PgPool, email_lc: &str) -> anyhow::Result<Option<i64>> {
    sqlx::query_scalar(
        "SELECT CEIL(EXTRACT(EPOCH FROM (locked_until - now())))::BIGINT
         FROM login_failures WHERE email_lc = $1 AND locked_until > now()",
    )
    .bind(email_lc)
    .fetch_optional(pool)
    .await
    .context("db: login lock")
}

/// Counts one more failure (a run that went quiet for a day starts over),
/// locks for `lock` if the policy says so, and returns the count.
pub async fn record(pool: &PgPool, email_lc: &str, lock: impl Fn(i32) -> Option<Duration>) -> anyhow::Result<i32> {
    let failures: i32 = sqlx::query_scalar(
        "INSERT INTO login_failures (email_lc, failures, last_failure_at) VALUES ($1, 1, now())
         ON CONFLICT (email_lc) DO UPDATE SET
            failures = CASE WHEN login_failures.last_failure_at < now() - interval '24 hours'
                            THEN 1 ELSE login_failures.failures + 1 END,
            last_failure_at = now()
         RETURNING failures",
    )
    .bind(email_lc)
    .fetch_one(pool)
    .await
    .context("db: record login failure")?;
    if let Some(wait) = lock(failures) {
        sqlx::query("UPDATE login_failures SET locked_until = now() + make_interval(secs => $2) WHERE email_lc = $1")
            .bind(email_lc)
            .bind(wait.as_secs_f64())
            .execute(pool)
            .await
            .context("db: lock login")?;
    }
    Ok(failures)
}

pub async fn clear(pool: &PgPool, email_lc: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM login_failures WHERE email_lc = $1")
        .bind(email_lc)
        .execute(pool)
        .await
        .context("db: clear login failures")?;
    Ok(())
}
