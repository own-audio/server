// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::auth::error::AuthError;
use chrono::{DateTime, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Claims embedded in a signed session JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    /// Subject — the user's UUID.
    pub sub: Uuid,

    /// Issued-at (Unix timestamp).
    pub iat: i64,

    /// Expiry (Unix timestamp).
    pub exp: i64,

    /// Session identifier, used for revocation checks.
    pub jti: Uuid,

    /// User role at issuance time.
    pub role: String,

    /// Refresh-token chain (device) this access token belongs to. Absent on
    /// tokens minted before refresh tokens existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Uuid>,
}

impl SessionClaims {
    pub fn new(user_id: Uuid, role: &str, ttl_secs: u64) -> Self {
        let now: DateTime<Utc> = Utc::now();
        Self {
            sub: user_id,
            iat: now.timestamp(),
            exp: now.timestamp() + ttl_secs as i64,
            jti: Uuid::new_v4(),
            role: role.to_owned(),
            chain: None,
        }
    }

    pub fn with_chain(mut self, chain_id: Uuid) -> Self {
        self.chain = Some(chain_id);
        self
    }
}

/// Sign a set of claims into a JWT string.
pub fn sign(claims: &SessionClaims, secret: &str) -> anyhow::Result<String> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| anyhow::anyhow!("jwt sign error: {e}"))
}

/// Verify and decode a JWT string.
pub fn verify(token: &str, secret: &str) -> Result<SessionClaims, AuthError> {
    let mut validation = Validation::default();
    validation.validate_exp = true;

    decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| {
        use jsonwebtoken::errors::ErrorKind;
        match e.kind() {
            ErrorKind::ExpiredSignature => AuthError::SessionInvalid,
            _ => AuthError::SessionInvalid,
        }
    })
}
