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

/// The v2 contract's revision: up by one with every addition, so a client can check for it.
/// Counted from 2026-11-17 (revision 1). Changes are additive only (API_COMPATIBILITY.md §4).
pub const V2_REVISION: u32 = 17;
/// `stable` since 1.0.4-beta.1, the release that completed docs/api-v2-completion-plan.md; it was
/// `draft` before, though additive only from revision 1 because the iPhone apps shipped on it.
pub const V2_STATUS: &str = "stable";

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
    /// The first run: whether the server has its first admin yet.
    pub setup: Setup,
    /// A public demo's shared sign-in, published on purpose; absent everywhere else. The demo
    /// account may listen but not change anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo: Option<Demo>,
}

#[derive(Serialize, ToSchema)]
pub struct Setup {
    /// `false` until the first admin exists; until then the console shows the setup wizard.
    pub complete: bool,
}

#[derive(Serialize, ToSchema)]
pub struct Demo {
    pub email: String,
    pub password: String,
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
    /// `POST /auth/password-reset` mails links; `false` on a server without mail.
    pub password_reset: bool,
    /// New addresses are confirmed by a mailed link; `false` on a server without mail, where
    /// every address counts as confirmed.
    pub email_verification: bool,
    /// The `grant_type`s `POST /auth/token` takes here.
    pub grants: Vec<String>,
    /// The providers' public client ids an app needs to start their sign-in itself: the
    /// desktop clients for the authorization-code grant, the web clients for a browser.
    pub clients: AuthClients,
}

#[derive(Serialize, ToSchema, Default)]
pub struct AuthClients {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google: Option<ProviderClients>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microsoft: Option<ProviderClients>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apple: Option<ProviderClients>,
}

#[derive(Serialize, ToSchema)]
pub struct ProviderClients {
    /// For the loopback authorization-code grant (RFC 8252); this server holds its secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop: Option<String>,
    /// For the provider's sign-in in a browser.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Limits {
    pub list_limit_default: i64,
    pub list_limit_max: i64,
    /// The largest single file `POST /uploads` takes.
    pub upload_max_bytes: i64,
    /// Plays, offline changes, things deleted at once: the most in one request.
    pub batch_max: usize,
    /// How long an `Idempotency-Key` is remembered.
    pub idempotency_hours: i32,
    /// How far back `/sync/changes` reaches.
    pub change_log_days: i64,
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
    let auth_cfg = &cfg.auth;
    let desktop = |id: Option<&str>, secret_ok: bool| crate::auth::non_empty(id).filter(|_| secret_ok);
    let google_desktop = auth_cfg.google.as_ref().filter(|g| g.enabled).and_then(|g| {
        desktop(g.desktop_client_id.as_deref(), crate::v2::auth::desktop_client(g.desktop_client_secret.as_deref()))
    });
    let microsoft_desktop =
        auth_cfg.microsoft.as_ref().filter(|m| m.enabled).and_then(|m| desktop(m.desktop_client_id.as_deref(), true));
    let mut grants = Vec::new();
    if providers.local {
        grants.push(crate::v2::auth::GRANT_PASSWORD);
    }
    grants.extend([crate::v2::auth::GRANT_REFRESH, crate::v2::auth::GRANT_DEVICE, crate::v2::auth::GRANT_MFA, crate::v2::auth::GRANT_TOKEN_EXCHANGE]);
    if !enabled.is_empty() {
        grants.push(crate::v2::auth::GRANT_ASSERTION);
    }
    if google_desktop.is_some() || microsoft_desktop.is_some() {
        grants.push(crate::v2::auth::GRANT_CODE);
    }
    let clients = AuthClients {
        google: providers.google.enabled.then(|| ProviderClients { desktop: google_desktop, web: providers.google.web_client_id.clone() }),
        microsoft: providers.microsoft.enabled.then_some(ProviderClients { desktop: microsoft_desktop, web: None }),
        apple: providers.apple.enabled.then(|| ProviderClients { desktop: None, web: providers.apple.web_client_id.clone() }),
    };
    let setup_complete = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(state.db())
        .await
        .unwrap_or(true);
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
            email_verification: crate::auth::verification::offered(cfg),
            grants: grants.into_iter().map(str::to_string).collect(),
            clients,
        },
        modules: {
            let mut modules = features(&state);
            // Push registration is kept (`/me/push-tokens`) but this server sends none: clients
            // read the inbox and the change feed.
            modules.insert("push".into(), Value::Bool(false));
            modules
        },
        addresses: addresses(cfg.server.base_url.as_deref(), cfg.server.addresses.as_deref()),
        limits: Limits {
            list_limit_default: DEFAULT_LIMIT,
            list_limit_max: MAX_LIMIT,
            upload_max_bytes: crate::uploads::MAX_UPLOAD_BYTES,
            batch_max: crate::v2::trash::BATCH_MAX,
            idempotency_hours: crate::v2::idempotency::RETENTION_HOURS,
            change_log_days: crate::db::changes::RETENTION_DAYS,
        },
        setup: Setup { complete: setup_complete },
        demo: cfg.server.demo.as_ref().map(|d| Demo { email: d.email.clone(), password: d.password.clone() }),
    })
}

