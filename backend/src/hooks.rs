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

use crate::db::access::Viewer;
use crate::storage::ObjectStore;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Whether a family may add to its storage ([`Hooks::storage_allowance`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageVerdict {
    Allowed,
    /// Why not, in words the client shows (`402`, `storage::quota`).
    Refused(String),
}

/// Whether an identify run may start ([`Hooks::identify_run_start`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifyVerdict {
    Allowed,
    /// Why not, in words the client shows (`402`).
    Refused(String),
}

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

/// A kind of item only an edition has — the hosted edition's `translation`, a podcast
/// episode translated and narrated for the household — that `/api/v2` still plays, keeps
/// progress on and sends in the change feed like its own kinds ([`Hooks::edition_kinds`]).
/// The core knows the kind by its name only; the edition answers every question about it.
#[async_trait]
pub trait EditionKind: Send + Sync + 'static {
    /// The kind's word in `/api/v2` (`translation`): in `POST /playback-info`, in
    /// `/me/progress/{kind}/{id}`, and in the change feed as `translation` and
    /// `progress.translation`.
    fn name(&self) -> &'static str;

    /// The stored audio of the item when the viewer may play it: its `media_objects` row and
    /// its length in seconds.
    async fn audio(&self, db: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<(Uuid, Option<i32>)>>;

    /// The current records of these items as the viewer sees them, for the change feed; an item
    /// left out is gone or no longer visible.
    async fn records(&self, db: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, Value)>>;

    /// Where the viewer is in the item; `None` when not started, unknown or not visible.
    async fn progress(&self, db: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<EditionProgress>>;

    /// The viewer's progress on this kind, most recently changed first, after a
    /// `(updated_at, id)` cursor, at most `limit`.
    async fn list_progress(
        &self,
        db: &PgPool,
        viewer: Viewer,
        unfinished: bool,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> anyhow::Result<Vec<EditionProgress>>;

    /// Save a position; the later one wins, as on the core's kinds. Answers what is stored now
    /// and whether this one was applied; `None` when the item is unknown or not visible.
    async fn put_progress(
        &self,
        db: &PgPool,
        viewer: Viewer,
        id: Uuid,
        position_ms: i64,
        finished: bool,
        at: DateTime<Utc>,
    ) -> anyhow::Result<Option<(EditionProgress, bool)>>;

    /// Start over.
    async fn delete_progress(&self, db: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<()>;
}

/// A position in an [`EditionKind`]'s item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditionProgress {
    pub item_id: Uuid,
    pub position_ms: i64,
    pub finished: bool,
    pub updated_at: DateTime<Utc>,
}

/// Something the caller's own work did, told on `GET /api/v2/events`
/// ([`Hooks::live_events`]): a narration or translation job that moved on.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveEvent {
    /// The event's name on the stream (`narration`, `translation`).
    pub event: &'static str,
    /// What it is about (the job); with `state`, what makes it news.
    pub id: Uuid,
    /// Told again only when this changes.
    pub state: String,
    /// The event's data.
    pub data: Value,
}

/// A household's credit for the server's administrator (`GET /api/v2/admin/families/{id}`).
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyCredit {
    /// Millionths of the currency's unit: the ledger counts storage by the day, in fractions
    /// of a cent.
    pub balance_micro: i64,
    pub currency: String,
    /// The last daily storage charge.
    pub last_charge: Option<CreditEntry>,
    /// The latest entries, newest first.
    pub entries: Vec<CreditEntry>,
}

/// One entry of a household's credit.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditEntry {
    pub id: Uuid,
    /// The ledger's word for it: `topup`, `storage_charge`, `adjustment`, …
    pub kind: String,
    pub amount_micro: i64,
    pub storage_bytes: Option<i64>,
    pub charge_date: Option<NaiveDate>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The edition kind of this name, if the edition has one.
pub fn edition_kind(hooks: &dyn Hooks, name: &str) -> Option<Arc<dyn EditionKind>> {
    hooks.edition_kinds().into_iter().find(|k| k.name() == name)
}

#[async_trait]
pub trait Hooks: Send + Sync + 'static {
    /// Reported by `GET /api/v1/server` for diagnostics. Clients never branch
    /// on it; they branch on `features`.
    fn edition(&self) -> &'static str {
        "foss"
    }

    /// One family per install: new accounts join it instead of founding their
    /// own, and nobody can be moved out into a second one. The default is
    /// many families, so an edition that forgets to answer never puts
    /// strangers into one family; the open-source binary answers `true`.
    fn one_family(&self) -> bool {
        false
    }

    /// Whether a family admin chooses where "identify this audiobook" looks
    /// (Open Library, Apple, or Google with the family's own key). Without the
    /// choice, Google Books with `GOOGLE_CLOUD__BOOKS_API_KEY` — what an edition
    /// that runs with its own key wants. The open-source binary answers `true`:
    /// keyless Google is turned away nearly everywhere.
    fn book_source_choice(&self) -> bool {
        false
    }

    /// A user exists for the first time and has just received their personal
    /// family (register, SSO sign-in, admin-create, first-admin setup). The
    /// hosted edition grants the once-ever welcome credit here.
    async fn user_created(&self, _db: &PgPool, _family_id: Uuid, _user_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }

    /// The user's email address is proven: by a provider, an invite sent to
    /// it, the admin who typed it, or the mailed link (`auth::verification`).
    /// Once per user. The hosted edition grants the welcome credit here, not
    /// at creation, so a run of fresh registrations earns nothing (H6).
    async fn email_verified(&self, _db: &PgPool, _family_id: Uuid, _user_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }

    /// [`email_verified`](Self::email_verified) when the proof is the mailed link, with what the
    /// confirmation page sent besides the link's secret: `human_check`, a token from a
    /// challenge such as Cloudflare Turnstile (the web console sends one when it is built with
    /// `VITE_TURNSTILE_SITEKEY`), and the caller's address. An edition may grant less to a
    /// confirmation without one; the address is confirmed either way. Once per user.
    async fn email_verified_by_link(
        &self,
        db: &PgPool,
        family_id: Uuid,
        user_id: Uuid,
        _human_check: Option<&str>,
        _ip: Option<&str>,
    ) -> anyhow::Result<()> {
        self.email_verified(db, family_id, user_id).await
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

    /// The most identifying these tracks could charge the family, in
    /// micro-currency: shown before a run starts. Free in the open-source
    /// edition.
    async fn identify_quote(&self, _db: &PgPool, _family_id: Uuid, _track_ids: &[Uuid]) -> anyhow::Result<i64> {
        Ok(0)
    }

    /// A run over these tracks is about to start. `Refused` stops it before
    /// anything is looked up; the hosted edition reserves the quote here.
    async fn identify_run_start(
        &self,
        _db: &PgPool,
        _family_id: Uuid,
        _user_id: Uuid,
        _run_id: Uuid,
        _track_ids: &[Uuid],
    ) -> anyhow::Result<IdentifyVerdict> {
        Ok(IdentifyVerdict::Allowed)
    }

    /// The run ended — every group looked at, failed, or deleted — while its
    /// proposals still exist. Called before the run's status says so, so a
    /// failure here is retried; it may be called more than once for a run.
    async fn identify_run_end(&self, _db: &PgPool, _run_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }

    /// May the family add `incoming_bytes` (0 when the size is not known
    /// yet)? Asked before a presigned upload is issued and again when it is
    /// completed, and before an episode is stored (security hardening plan
    /// S1). The open-source edition always allows — a per-family quota lives
    /// in the core itself (`STORAGE__FAMILY_QUOTA_BYTES`, checked before this
    /// is asked); the hosted edition refuses a family whose credit is gone.
    async fn storage_allowance(&self, _db: &PgPool, _family_id: Uuid, _incoming_bytes: i64) -> anyhow::Result<StorageVerdict> {
        Ok(StorageVerdict::Allowed)
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

    /// The kinds of item only this edition has ([`EditionKind`]).
    fn edition_kinds(&self) -> Vec<Arc<dyn EditionKind>> {
        Vec::new()
    }

    /// The caller's own work as it stands, polled by `GET /api/v2/events` every few seconds:
    /// each one is told when its `state` changes. Include what ended lately, so the end is told.
    async fn live_events(&self, _db: &PgPool, _user_id: Uuid) -> anyhow::Result<Vec<LiveEvent>> {
        Ok(Vec::new())
    }

    /// A household's credit, for `GET /api/v2/admin/families/{id}`; `None` where there is no
    /// money.
    async fn family_credit(&self, _db: &PgPool, _family_id: Uuid) -> anyhow::Result<Option<FamilyCredit>> {
        Ok(None)
    }

    /// Every household's credit together, in millionths, with its currency, for
    /// `GET /api/v2/admin/stats`; `None` where there is no money.
    async fn credit_total(&self, _db: &PgPool) -> anyhow::Result<Option<(i64, String)>> {
        Ok(None)
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
impl Hooks for NoopHooks {
    fn one_family(&self) -> bool {
        true
    }

    fn book_source_choice(&self) -> bool {
        true
    }
}

/// How a binary chooses its edition: a function from the loaded config to
/// the hooks, so the hooks can read the same environment the core does.
pub type HooksFactory = Box<dyn FnOnce(&crate::app::AppConfig) -> Arc<dyn Hooks> + Send>;

pub fn noop_factory() -> HooksFactory {
    Box::new(|_| Arc::new(NoopHooks))
}
