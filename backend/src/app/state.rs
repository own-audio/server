// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::app::AppConfig;
use crate::auth::at_rest::Cipher;
use crate::hooks::Hooks;
use crate::storage::ObjectStore;
use sqlx::PgPool;
use std::sync::Arc;

/// Shared application state threaded through Axum handlers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    pub config: AppConfig,
    pub db: PgPool,
    pub storage: ObjectStore,
    pub hooks: Arc<dyn Hooks>,
    pub at_rest: Cipher,
}

impl AppState {
    pub fn new(config: AppConfig, db: PgPool, storage: ObjectStore, hooks: Arc<dyn Hooks>) -> Self {
        let at_rest = Cipher::derive(&config.auth.session_secret, "subsonic-api-key");
        Self {
            inner: Arc::new(Inner { config, db, storage, hooks, at_rest }),
        }
    }

    /// Encrypts what the database must not hold in the clear (`auth::at_rest`).
    pub fn at_rest(&self) -> &Cipher {
        &self.inner.at_rest
    }

    /// The edition seam — see `crate::hooks`.
    pub fn hooks(&self) -> &Arc<dyn Hooks> {
        &self.inner.hooks
    }

    pub fn config(&self) -> &AppConfig {
        &self.inner.config
    }

    pub fn db(&self) -> &PgPool {
        &self.inner.db
    }

    pub fn storage(&self) -> &ObjectStore {
        &self.inner.storage
    }
}
