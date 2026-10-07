// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod config;
pub mod state;

pub use config::{AppConfig, AuthConfig, LibraryConfig, ServerConfig, StorageConfig};
pub use state::AppState;

use crate::db;
use crate::hooks::HooksFactory;
use crate::http::router;
use crate::jobs;
use crate::observability;
use crate::storage;
use anyhow::Context;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::signal;
use tracing::info;

/// The session secret signs every sign-in and media link: anyone who guesses
/// it can mint an admin session. A short or placeholder one stops the start;
/// a merely weak one is logged loudly.
fn check_session_secret(secret: &str) -> anyhow::Result<()> {
    let s = secret.trim();
    if s.len() < 16 || s.eq_ignore_ascii_case("change_me") || s.eq_ignore_ascii_case("changeme") {
        anyhow::bail!(
            "AUTH__SESSION_SECRET (SESSION_SECRET in .env) is missing, a placeholder or shorter than 16 characters; \
             set it to a random value: openssl rand -hex 32"
        );
    }
    if s.len() < 32 || s.to_ascii_lowercase().contains("changeme") {
        tracing::warn!("AUTH__SESSION_SECRET looks weak; use a random value of 32 characters or more (openssl rand -hex 32)");
    }
    Ok(())
}

/// Bootstrap the application: load config, connect to DB, storage, workers.
///
/// `hooks` turns the loaded config into the edition's [`crate::hooks::Hooks`]
/// (see `crate::hooks::noop_factory` for the open-source edition). Returns
/// `(AppState, AppConfig)` so a hosted binary can merge its own routes
/// before calling [`router::finalize`].
pub async fn bootstrap(hooks: HooksFactory) -> anyhow::Result<(AppState, AppConfig)> {
    let config = AppConfig::load().context("failed to load configuration")?;

    observability::init(&config.log_level);
    check_session_secret(&config.auth.session_secret)?;

    info!(
        host = %config.server.host,
        port = %config.server.port,
        "starting audio2"
    );

    // Connect to PostgreSQL
    let pool = db::connect(
        &config.database_url,
        config.database_max_connections.unwrap_or(db::DEFAULT_MAX_CONNECTIONS),
    )
        .await
        .context("failed to connect to database")?;

    // Run pending migrations
    db::migrate(&pool).await.context("failed to run migrations")?;

    // Seed a dev admin if no users exist yet — opt-in only. On a reachable
    // instance this would publish a documented admin@audio2.local / admin
    // login, and it also pre-empts the /setup/complete first-admin flow.
    if config.auth.dev_seed_admin {
        seed_dev_admin(&pool).await;
    }

    // Build storage client
    let media_links = storage::MediaLinks::new(&config.auth.session_secret, config.server.base_url.as_deref());
    let object_store = storage::connect(&config.storage, media_links)
        .await
        .context("failed to connect to object storage")?;

    let hooks = hooks(&config);

    // Assemble shared application state
    let state = AppState::new(config.clone(), pool.clone(), object_store.clone(), hooks.clone());

    // Read-only library folders: register them and scan in the background.
    match crate::library_folders::configured(&config) {
        Ok(folders) if !folders.is_empty() => crate::library_folders::spawn(state.clone()),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "library folders: configuration ignored"),
    }

    // Start background job worker
    jobs::worker::spawn(pool, object_store, config.clone(), hooks);

    Ok((state, config))
}

/// Start the server with only core routes and the given edition hooks.
pub async fn run(hooks: HooksFactory) -> anyhow::Result<()> {
    let (state, config) = bootstrap(hooks).await?;
    serve(router::build(state), &config).await
}

/// Bind and serve an assembled router until SIGTERM/Ctrl-C. A hosted binary
/// calls this with its own router from [`router::finalize`].
pub async fn serve(app: axum::Router, config: &AppConfig) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .context("invalid server address")?;

    let listener = TcpListener::bind(addr)
        .await
        .context("failed to bind TCP listener")?;

    info!("listening on {addr}");

    // Peer addresses for the per-IP rate limits (`http::rate_limit`).
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    info!("server shut down cleanly");
    Ok(())
}

/// On first startup, create a default admin user so the app is usable immediately.
///
/// Credentials: admin@audio2.local / admin
///
/// Only runs when `AUTH__DEV_SEED_ADMIN=true` and the `users` table is empty.
/// Never enable it on a reachable instance.
async fn seed_dev_admin(pool: &sqlx::PgPool) {
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    use argon2::Argon2;

    match db::users::list_all(pool).await {
        Ok(users) if !users.is_empty() => return,
        Err(e) => {
            tracing::warn!("could not check users table: {e}");
            return;
        }
        Ok(_) => {}
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = match Argon2::default().hash_password(b"admin", &salt) {
        Ok(h) => h.to_string(),
        Err(e) => {
            tracing::error!("failed to hash dev admin password: {e}");
            return;
        }
    };

    let user = match db::users::insert(pool, "admin@audio2.local", "Admin", "admin").await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("failed to create dev admin user: {e}");
            return;
        }
    };

    if let Err(e) = db::users::insert_local_identity(pool, user.id, &hash).await {
        tracing::error!("failed to create dev admin identity: {e}");
        return;
    }

    tracing::warn!(
        email = "admin@audio2.local",
        password = "admin",
        "⚠  seeded DEV admin user — change credentials before going to production"
    );
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("received shutdown signal");
}

#[cfg(test)]
mod tests {
    use super::check_session_secret;

    #[test]
    fn short_or_placeholder_secrets_stop_the_start() {
        assert!(check_session_secret("").is_err());
        assert!(check_session_secret("CHANGE_ME").is_err());
        assert!(check_session_secret("short-secret").is_err());
        assert!(check_session_secret("changeme-replace-in-production").is_ok());
        assert!(check_session_secret(&"a1".repeat(32)).is_ok());
    }
}
