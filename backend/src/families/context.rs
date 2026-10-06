// SPDX-License-Identifier: AGPL-3.0-or-later
/// Axum extractor resolving the caller's family membership.
///
/// Use it in any handler that reads or writes family-scoped data:
/// ```ignore
/// async fn my_handler(family: FamilyContext, ...) -> impl IntoResponse { ... }
/// ```
///
/// Membership is resolved (and lazily created for accounts that predate
/// families) once per request, so handlers can assume `family_id` exists.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct FamilyContext {
    pub user_id: Uuid,
    pub family_id: Uuid,
    /// `family_admin` or `member` — scoped to this family, independent of the
    /// instance-wide `users.role`.
    pub family_role: String,
    /// True when the caller is an instance administrator. Instance admins are
    /// treated as family admins for management purposes.
    pub is_global_admin: bool,
    /// May add content to the family library / generate narration. Admin-set
    /// per member — see docs/family-permissions-plan.md.
    pub can_upload: bool,
    pub can_generate: bool,
}

impl FamilyContext {
    pub fn is_family_admin(&self) -> bool {
        self.is_global_admin || self.family_role == "family_admin"
    }

    /// The parameters every content-visibility query needs.
    pub fn viewer(&self) -> db::access::Viewer {
        db::access::Viewer {
            user_id: self.user_id,
            family_id: self.family_id,
            is_family_admin: self.is_family_admin(),
        }
    }

    /// What the caller may actually do, admin override included — the same
    /// rule `require_can_upload` enforces, exposed so responses and guards
    /// cannot drift apart.
    pub fn effective_can_upload(&self) -> bool {
        self.is_family_admin() || self.can_upload
    }

    pub fn effective_can_generate(&self) -> bool {
        self.is_family_admin() || self.can_generate
    }

    /// The upload gate. Lives on the context rather than in each handler so a
    /// new upload route cannot quietly ship without it. Family admins are
    /// always allowed — they are the ones handing the permission out.
    pub fn require_can_upload(&self) -> Result<(), AuthError> {
        if self.effective_can_upload() {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }

    pub fn require_can_generate(&self) -> Result<(), AuthError> {
        if self.effective_can_generate() {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }

    pub fn require_family_admin(&self) -> Result<(), AuthError> {
        if self.is_family_admin() {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }
}

impl FromRequestParts<AppState> for FamilyContext {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth = AuthUser::from_request_parts(parts, state).await?;

        let membership = db::families::ensure_membership(state.db(), auth.user_id)
            .await
            .map_err(AuthError::Internal)?;

        Ok(FamilyContext {
            user_id: auth.user_id,
            family_id: membership.family_id,
            family_role: membership.role,
            is_global_admin: auth.role == "admin",
            can_upload: membership.can_upload,
            can_generate: membership.can_generate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(role: &str, can_upload: bool, can_generate: bool) -> FamilyContext {
        FamilyContext {
            user_id: Uuid::nil(),
            family_id: Uuid::nil(),
            family_role: role.to_string(),
            is_global_admin: false,
            can_upload,
            can_generate,
        }
    }

    #[test]
    fn a_member_without_the_flag_is_refused() {
        assert!(ctx("member", false, false).require_can_upload().is_err());
        assert!(ctx("member", false, false).require_can_generate().is_err());
    }

    #[test]
    fn a_member_with_the_flag_is_allowed() {
        assert!(ctx("member", true, false).require_can_upload().is_ok());
        assert!(ctx("member", false, true).require_can_generate().is_ok());
    }

    /// Admins hand the permission out, so withholding it from them would only
    /// let a family lock itself out of its own library.
    #[test]
    fn a_family_admin_may_always_upload_and_generate() {
        let admin = ctx("family_admin", false, false);
        assert!(admin.require_can_upload().is_ok());
        assert!(admin.require_can_generate().is_ok());
    }

    #[test]
    fn an_instance_admin_is_treated_the_same() {
        let mut global = ctx("member", false, false);
        global.is_global_admin = true;
        assert!(global.require_can_upload().is_ok());
        assert!(global.require_can_generate().is_ok());
    }

    #[test]
    fn the_reported_permission_matches_what_the_guard_enforces() {
        for c in [ctx("member", false, false), ctx("member", true, true), ctx("family_admin", false, false)] {
            assert_eq!(c.effective_can_upload(), c.require_can_upload().is_ok());
            assert_eq!(c.effective_can_generate(), c.require_can_generate().is_ok());
        }
    }
}
