// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

/// This server's identity, made once by migration 0092 and never changed.
pub async fn id(pool: &PgPool) -> anyhow::Result<Uuid> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM instance")
        .fetch_one(pool)
        .await
        .context("db: instance id")
}
