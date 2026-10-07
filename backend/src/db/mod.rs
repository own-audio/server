// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

pub mod users;
pub mod sessions;
pub mod refresh_tokens;
pub mod device_auth;
pub mod families;
pub mod access;
pub mod stats;
pub mod sync;
pub mod podcasts;
pub mod audiobooks;
pub mod authors;
pub mod storage_usage;
pub mod collections;
pub mod playback;
pub mod jobs;
pub mod media;
pub mod music;
pub mod subsonic;
pub mod trash;

/// Pool size when `DATABASE_MAX_CONNECTIONS` is unset. A family's requests,
/// the worker and file sync fit in ten; every idle PostgreSQL connection costs
/// a few MB of the database's memory (`docs/RAM_USAGE.md`).
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;

/// Connect to PostgreSQL and return a connection pool.
pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
        .context("could not connect to PostgreSQL")
}

/// Run all pending SQLx migrations from the `migrations/` directory.
///
/// **Tolerates a schema newer than this binary**, which is what makes a
/// rollback survivable. sqlx's default is to refuse to start when the database
/// carries an applied migration the binary does not know about, and migrations
/// here are forward-only: rolling the image back therefore left the old binary
/// facing a schema it could not validate, and it exited on boot. On 2026-08-29
/// that turned one failed canary smoke-test assertion into a crash loop that
/// took manual intervention to clear — the rollback step, whose whole purpose
/// is to restore service, was guaranteed to destroy it for any deploy that
/// added a migration.
///
/// The cost of tolerating it is that a binary pointed at the *wrong* database
/// no longer fails loudly on that basis, so the mismatch is logged instead:
/// running ahead of your own migrations is survivable but never silent.
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    let mut migrator = sqlx::migrate!("./migrations");
    warn_if_schema_is_ahead(pool, &migrator).await;
    migrator.set_ignore_missing(true);
    migrator
        .run(pool)
        .await
        .context("database migration failed")?;
    tracing::info!("database migrations applied");
    Ok(())
}

/// Log any migration the database has applied that this binary does not carry.
///
/// Best-effort by design: on a fresh database `_sqlx_migrations` does not exist
/// yet, and a diagnostic must never be the reason startup fails.
async fn warn_if_schema_is_ahead(pool: &PgPool, migrator: &sqlx::migrate::Migrator) {
    let applied: Vec<i64> = match sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(_) => return,
    };

    let known: std::collections::HashSet<i64> = migrator.iter().map(|m| m.version).collect();
    let unknown: Vec<i64> = applied.into_iter().filter(|v| !known.contains(v)).collect();
    if !unknown.is_empty() {
        tracing::warn!(
            migrations = ?unknown,
            "database has migrations this build does not carry — running against a schema \
             newer than this binary, which is expected immediately after a rollback"
        );
    }
}
