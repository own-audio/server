// SPDX-License-Identifier: AGPL-3.0-or-later
//! The seam between the open-source core and the hosted edition.
//!
//! Everything a hosted deployment adds — the credit ledger, payments, the
//! narration and translation pipelines, operator dashboards — reaches the
//! core through this trait and nothing else. The methods are deliberately
//! concrete (a trash-restore charge, not a generic "spend"): the only money
//! the core itself ever touches is that one charge, and the pipelines that
//! charge for narration live entirely in the hosted edition. The core calls these methods
//! at the points where such an edition needs a say; [`NoopHooks`] is what
//! the open-source binary installs and answers "nothing to add, always
//! allowed, for free". There is deliberately no `edition` value anywhere in
//! the core: the hook *is* the branch.
//!
//! See `docs/foss-seam-plan.md` for the step-by-step refactor behind this and
//! `own-audio-foss/docs/API_COMPATIBILITY.md` for the `features` contract.

use crate::storage::ObjectStore;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// A background job executor contributed by an edition. The worker consults
/// the handlers from [`Hooks::job_handlers`] before its own job types.
#[async_trait]
pub trait JobHandler: Send + Sync + 'static {
    async fn run(
        &self,
        db: &PgPool,
        storage: &ObjectStore,
        job: &crate::jobs::models::Job,
    ) -> anyhow::Result<()>;
}

#[async_trait]
pub trait Hooks: Send + Sync + 'static {
    /// Reported by `GET /api/v1/server` for diagnostics. Clients never branch
    /// on it; they branch on `features`.
    fn edition(&self) -> &'static str {
        "foss"
    }

    /// A user exists for the first time and has just received their personal
    /// family (register, SSO sign-in, admin-create, first-admin setup). The
    /// hosted edition grants the once-ever welcome credit here.
    async fn user_created(&self, _db: &PgPool, _family_id: Uuid, _user_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }

    /// What restoring `size_bytes` after `days` whole days in the trash would
    /// charge the family, in micro-currency; shown to the user before they
    /// confirm. Nothing is charged in the open-source edition.
    async fn trash_restore_quote(&self, _db: &PgPool, _size_bytes: i64, _days: i64) -> anyhow::Result<i64> {
        Ok(0)
    }

    /// A restore happened; charge for it. Returns what was actually charged
    /// (the hosted edition clamps to the balance).
    async fn trash_restore_charge(
        &self,
        _db: &PgPool,
        _family_id: Uuid,
        _user_id: Uuid,
        _size_bytes: i64,
        _days: i64,
        _note: &str,
    ) -> anyhow::Result<i64> {
        Ok(0)
    }

    /// Extra fields merged into `GET /api/v1/admin/stats`.
    async fn dashboard_extras(&self, _db: &PgPool, _storage: &ObjectStore) -> anyhow::Result<Map<String, Value>> {
        Ok(Map::new())
    }

    /// Per-family credit balances for the instance-admin family list
    /// (`balance_micro`); families absent from the map show 0.
    async fn family_balances(&self, _db: &PgPool) -> anyhow::Result<HashMap<Uuid, i64>> {
        Ok(HashMap::new())
    }

    /// Extra fields merged into the instance-admin family detail
    /// (`balance_micro`, `last_charge`, `entries` in the hosted edition).
    async fn family_extras(
        &self,
        _db: &PgPool,
        _storage: &ObjectStore,
        _family_id: Uuid,
    ) -> anyhow::Result<Map<String, Value>> {
        Ok(Map::new())
    }

    /// Job types this edition executes, keyed by `job_type`.
    fn job_handlers(&self) -> Vec<(&'static str, Arc<dyn JobHandler>)> {
        Vec::new()
    }

    /// Called once per worker-loop pass; the place to enqueue an edition's
    /// own scheduled work (the hosted edition's daily storage charge).
    async fn worker_tick(&self, _db: &PgPool, _storage: &ObjectStore, _now: DateTime<Utc>) -> anyhow::Result<()> {
        Ok(())
    }

    /// Feature flags this edition adds to `GET /api/v1/server` (`billing`,
    /// `payments`, `narration`, `translation`). Keys absent here read as `false`.
    fn features(&self) -> Map<String, Value> {
        Map::new()
    }
}

/// The open-source edition: nothing to add, everything allowed, for free.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopHooks;

#[async_trait]
impl Hooks for NoopHooks {}

/// How a binary chooses its edition: a function from the loaded config to
/// the hooks, so the hooks can read the same environment the core does.
pub type HooksFactory = Box<dyn FnOnce(&crate::app::AppConfig) -> Arc<dyn Hooks> + Send>;

pub fn noop_factory() -> HooksFactory {
    Box::new(|_| Arc::new(NoopHooks))
}
