// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::app::AppConfig;
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
}

impl AppState {
    pub fn new(config: AppConfig, db: PgPool, storage: ObjectStore, hooks: Arc<dyn Hooks>) -> Self {
        Self {
            inner: Arc::new(Inner { config, db, storage, hooks }),
        }
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
