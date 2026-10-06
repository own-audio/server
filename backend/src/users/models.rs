// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    /// Set via `POST /users/me/avatar`; served back through `GET /users/{id}/avatar`.
    pub avatar_object_id: Option<Uuid>,
    /// Whether this user has switched on listening-derived recommendations.
    /// Off for everyone until they do (migration 0061). Per user — a family
    /// admin cannot set it for a member.
    #[sqlx(default)]
    pub recommendations_enabled: bool,
    #[sqlx(default)]
    pub recommendations_changed_at: Option<DateTime<Utc>>,
    /// Languages podcast discovery answers in, as base subtags (`en`, `cs`).
    /// Empty means every language — see migration 0063.
    #[sqlx(default)]
    pub discovery_languages: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuthIdentity {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub provider_subject: Option<String>,
    /// Only present when selecting for authentication; excluded from public APIs.
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

impl Session {
    pub fn is_valid(&self) -> bool {
        self.revoked_at.is_none() && self.expires_at > Utc::now()
    }
}
