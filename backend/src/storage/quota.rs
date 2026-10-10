// SPDX-License-Identifier: AGPL-3.0-or-later
//! Room for new storage (security hardening plan S1, C1, C2).
//!
//! Every path that adds bytes to a family's storage — a presigned upload,
//! its completion, a stored podcast episode — asks here first. Two answers
//! combine: the core's own per-family quota (`STORAGE__FAMILY_QUOTA_BYTES`,
//! unset means none), and the edition's say through
//! [`Hooks::storage_allowance`], where the hosted edition refuses a family
//! whose credit is used up. Both refuse with `402` and a sentence the client
//! can show.
use crate::app::{AppConfig, AppState};
use crate::auth::error::AuthError;
use crate::hooks::{Hooks, StorageVerdict};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// What a refusal looks like inside an `anyhow` chain, so a function that
/// returns `anyhow::Result` (the episode download, shared with the worker)
/// can still be answered as `402` by a handler — see [`to_auth_error`].
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refused(pub String);

#[derive(Clone)]
pub struct Guard {
    quota_bytes: Option<i64>,
    hooks: Arc<dyn Hooks>,
}

impl Guard {
    pub fn new(config: &AppConfig, hooks: Arc<dyn Hooks>) -> Self {
        Self { quota_bytes: config.storage.family_quota_bytes.filter(|q| *q > 0), hooks }
    }

    pub fn for_state(state: &AppState) -> Self {
        Self::new(state.config(), state.hooks().clone())
    }

    /// `Ok` when the family may add `incoming_bytes`; `incoming_bytes` is 0
    /// when the size is not known yet, which still refuses a family already
    /// over its limit.
    pub async fn check(&self, db: &PgPool, bucket: &str, family_id: Uuid, incoming_bytes: i64) -> anyhow::Result<()> {
        if let Some(quota) = self.quota_bytes {
            let stored = crate::db::storage_usage::family_storage(db, bucket, family_id).await?.total_bytes;
            if let Some(reason) = over_quota(quota, stored, incoming_bytes) {
                return Err(Refused(reason).into());
            }
        }
        match self.hooks.storage_allowance(db, family_id, incoming_bytes).await? {
            StorageVerdict::Allowed => Ok(()),
            StorageVerdict::Refused(reason) => Err(Refused(reason).into()),
        }
    }
}

fn over_quota(quota: i64, stored: i64, incoming: i64) -> Option<String> {
    if stored.saturating_add(incoming.max(0)) <= quota {
        return None;
    }
    let free = (quota - stored).max(0);
    Some(if incoming > 0 {
        format!("the family's storage limit is reached: {incoming} bytes do not fit in the {free} bytes left of {quota}")
    } else {
        format!("the family's storage limit of {quota} bytes is reached ({stored} bytes stored)")
    })
}

/// A refusal becomes `402`; anything else stays a server error.
pub fn to_auth_error(error: anyhow::Error) -> AuthError {
    match error.chain().find_map(|e| e.downcast_ref::<Refused>()) {
        Some(refused) => AuthError::StorageRefused(refused.0.clone()),
        None => AuthError::Internal(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_until_the_limit_and_not_past_it() {
        assert_eq!(over_quota(100, 60, 40), None);
        assert!(over_quota(100, 60, 41).is_some());
        assert_eq!(over_quota(100, 100, 0), None);
        assert!(over_quota(100, 101, 0).is_some());
        // A negative "incoming" (unknown, defensively) never frees room.
        assert_eq!(over_quota(100, 100, -5), None);
    }

    #[test]
    fn a_refusal_is_a_402_through_a_context_chain() {
        let err = anyhow::Error::from(Refused("no room".into())).context("fetch audio");
        assert!(matches!(to_auth_error(err), AuthError::StorageRefused(m) if m == "no room"));
        assert!(matches!(to_auth_error(anyhow::anyhow!("boom")), AuthError::Internal(_)));
    }
}
