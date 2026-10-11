// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /.well-known/own-audio` (RFC 8615): what this server is and offers, for
//! a client that has only an address. Public and cheap; it replaces
//! `/api/v1/server` for v2 clients (DESIGN §10).

use crate::app::AppState;
use crate::http::server_info::{API_REVISION, addresses, features};
use crate::v2::page::{DEFAULT_LIMIT, MAX_LIMIT};
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

/// The v2 contract's revision; `status` stays `draft` until our apps ship on it.
pub const V2_REVISION: u32 = 0;
pub const V2_STATUS: &str = "draft";

#[derive(Serialize, ToSchema)]
pub struct Discovery {
    /// The server's stable id: a client trusts a server by it, whichever address answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    /// `core` for the open-source server; editions name themselves.
    pub edition: String,
    pub version: String,
    /// Every API this server speaks.
    pub api: Vec<ApiVersion>,
    pub auth: AuthMethods,
    /// Optional capabilities, one key each; a missing key reads as `false`.
    #[schema(value_type = Object)]
    pub modules: Map<String, Value>,
    /// Where the server answers, `scope` one of `public`, `lan`, `vpn`. A client
    /// races them and keeps the fastest that answers with the same `id`.
    #[schema(value_type = Vec<Object>)]
    pub addresses: Value,
    pub limits: Limits,
}

#[derive(Serialize, ToSchema)]
pub struct ApiVersion {
    pub version: u32,
    pub path: String,
    pub revision: u32,
    /// `stable`, or `draft` while the version may still change in any way.
    pub status: String,
}

#[derive(Serialize, ToSchema)]
pub struct AuthMethods {
    /// Email and password (`grant_type=password`).
    pub password: bool,
    /// Identity providers whose id token the server accepts.
    pub providers: Vec<String>,
    /// TVs and cars: the device authorization grant (RFC 8628).
    pub device_grant: bool,
    /// A second factor from an authenticator app.
    pub totp: bool,
    pub password_reset: bool,
}

#[derive(Serialize, ToSchema)]
pub struct Limits {
    pub list_limit_default: i64,
    pub list_limit_max: i64,
}

/// What this server is and offers.
#[utoipa::path(get, path = "/.well-known/own-audio", tag = "discovery",
    extensions(("x-permission" = json!("public"))),
    responses((status = 200, body = Discovery)))]
pub async fn well_known(State(state): State<AppState>) -> Json<Discovery> {
    let cfg = state.config();
    let providers = crate::auth::providers_response(&cfg.auth);
    let mut enabled = Vec::new();
    for (name, on) in [("apple", providers.apple.enabled), ("google", providers.google.enabled), ("microsoft", providers.microsoft.enabled)] {
        if on {
            enabled.push(name.to_string());
        }
    }
    Json(Discovery {
        id: crate::db::instance::id(state.db()).await.ok().map(|id| id.to_string()),
        name: "own.audio".into(),
        edition: state.hooks().edition().to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
        api: vec![
            ApiVersion { version: 1, path: "/api/v1".into(), revision: API_REVISION, status: "stable".into() },
            ApiVersion { version: 2, path: "/api/v2".into(), revision: V2_REVISION, status: V2_STATUS.into() },
        ],
        auth: AuthMethods {
            password: providers.local,
            providers: enabled,
            device_grant: true,
            totp: true,
            password_reset: crate::auth::password_reset_offered(cfg),
        },
        modules: features(&state),
        addresses: addresses(cfg.server.base_url.as_deref(), cfg.server.addresses.as_deref()),
        limits: Limits { list_limit_default: DEFAULT_LIMIT, list_limit_max: MAX_LIMIT },
    })
}

