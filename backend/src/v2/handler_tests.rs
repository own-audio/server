// SPDX-License-Identifier: AGPL-3.0-or-later
//! Handler tests through the real `/api/v2` router (docs/api-v2-completion-plan.md §4.1):
//! what the handlers decide before the database is asked — validation and its problem codes,
//! the grants' checks, the rate-limit buckets on the routes that take a password or a code,
//! and refusing what a server without mail cannot do. The pool points nowhere; a test that
//! reached it would fail. What needs the database is the conformance suite's.

use crate::app::{AppConfig, AppState};
use crate::http::rate_limit::Limiters;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn state(overrides: Value) -> AppState {
    let media = tempfile::tempdir().unwrap().keep();
    let mut config = json!({
        "server": {},
        "database_url": "postgres://nobody@127.0.0.1:9/none",
        "storage": { "kind": "local", "path": media.to_string_lossy() },
        "auth": { "session_secret": "handler-tests-secret-0123456789abcdef" },
    });
    merge(&mut config, overrides);
    let config: AppConfig = serde_json::from_value(config).expect("a test configuration");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy(&config.database_url)
        .unwrap();
    let links = crate::storage::MediaLinks::new(&config.auth.session_secret, None);
    let storage = crate::storage::connect(&config.storage, links).await.unwrap();
    let hooks = crate::hooks::noop_factory()(&config);
    AppState::new(config, pool, storage, hooks)
}

fn merge(into: &mut Value, from: Value) {
    match (into, from) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, v) => *slot = v,
    }
}

fn app(state: &AppState, limits: &Limiters) -> axum::Router {
    axum::Router::new().nest("/api/v2", crate::v2::mount(limits, state, axum::Router::new())).with_state(state.clone())
}

async fn send(app: &axum::Router, method: &str, path: &str, body: Value, form: bool) -> (StatusCode, Value) {
    let (content_type, bytes) = if form {
        let pairs: Vec<(String, String)> =
            body.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect();
        ("application/x-www-form-urlencoded", serde_urlencoded(&pairs))
    } else {
        ("application/json", serde_json::to_vec(&body).unwrap())
    };
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(bytes))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let raw = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let json = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    if status.is_client_error() || status.is_server_error() {
        assert!(content.starts_with(crate::v2::problem::CONTENT_TYPE), "{method} {path} → {status} answered {content}");
    }
    (status, json)
}

fn serde_urlencoded(pairs: &[(String, String)]) -> Vec<u8> {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish().into_bytes()
}

const GOOD: &str = "correct horse battery staple";

#[tokio::test]
async fn sign_up_refuses_with_a_code_for_each_reason() {
    let closed = state(json!({})).await;
    let app = app(&closed, &Limiters::disabled());
    let body = |email: &str, password: &str, name: &str| json!({ "email": email, "password": password, "display_name": name });

    let (s, p) = send(&app, "POST", "/api/v2/auth/register", body("not-an-address", GOOD, "A"), false).await;
    assert_eq!((s, p["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_email")));
    let (s, p) = send(&app, "POST", "/api/v2/auth/register", body("a@example.com", "short", "A"), false).await;
    assert_eq!((s, p["code"].as_str()), (StatusCode::BAD_REQUEST, Some("weak_password")));
    assert_eq!(p["errors"][0]["field"], "/password");
    let (s, p) = send(&app, "POST", "/api/v2/auth/register", body("a@example.com", GOOD, "  "), false).await;
    assert_eq!((s, p["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_display_name")));
    let (s, p) = send(&app, "POST", "/api/v2/auth/register", body("a@example.com", GOOD, "Anna"), false).await;
    assert_eq!((s, p["code"].as_str()), (StatusCode::FORBIDDEN, Some("registration_closed")));
    let (s, _) = send(&app, "POST", "/api/v2/auth/register", json!({ "email": "a@example.com" }), false).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "a body without the fields is refused before anything else");
}

#[tokio::test]
async fn mail_flows_say_so_on_a_server_without_mail() {
    let state = state(json!({})).await;
    let app = app(&state, &Limiters::disabled());
    let (s, p) = send(&app, "POST", "/api/v2/auth/password-reset", json!({ "email": "a@example.com" }), false).await;
    assert_eq!((s, p["code"].as_str(), p["feature"].as_str()), (StatusCode::NOT_IMPLEMENTED, Some("feature_unavailable"), Some("password_reset")));
    let (s, p) = send(&app, "POST", "/api/v2/auth/password-reset/confirm", json!({ "token": "x", "password": "short" }), false).await;
    assert_eq!((s, p["code"].as_str()), (StatusCode::BAD_REQUEST, Some("weak_password")), "the password rule comes before the link");
}

#[tokio::test]
async fn the_password_and_code_routes_share_the_sign_in_bucket() {
    let state = state(json!({})).await;
    let register = json!({ "email": "a@example.com", "password": GOOD, "display_name": "Anna" });
    let routes = [
        ("/api/v2/auth/register", register),
        ("/api/v2/auth/password-reset", json!({ "email": "a@example.com" })),
        ("/api/v2/auth/password-reset/confirm", json!({ "token": "x", "password": "short" })),
        ("/api/v2/auth/email-verification", json!({})),
        ("/api/v2/auth/token", json!({ "grant_type": "password" })),
    ];
    for (path, body) in &routes {
        // A bucket of two per address: the third try within the minute waits.
        let mut limits = Limiters::disabled();
        limits.login = crate::http::rate_limit::Limiter::per_minute(2, false);
        let app = app(&state, &limits);
        for _ in 0..2 {
            let (s, _) = send(&app, "POST", path, body.clone(), false).await;
            assert_ne!(s, StatusCode::TOO_MANY_REQUESTS, "{path} limited too early");
        }
        let (s, p) = send(&app, "POST", path, body.clone(), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{path}");
        // One bucket for all of them: a guess here costs a guess at signing in.
        let (s, _) = send(&app, "POST", "/api/v2/auth/token", json!({ "grant_type": "password" }), false).await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{path} and the token endpoint share the bucket");
    }
}

#[tokio::test]
async fn the_token_exchange_names_what_it_takes() {
    let state = state(json!({})).await;
    let app = app(&state, &Limiters::disabled());
    let grant = crate::v2::auth::GRANT_TOKEN_EXCHANGE;
    let (s, p) = send(&app, "POST", "/api/v2/auth/token", json!({ "grant_type": grant }), true).await;
    assert_eq!((s, p["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_request")));
    assert!(p["error_description"].as_str().unwrap().contains("subject_token"));
    let (s, p) = send(&app, "POST", "/api/v2/auth/token", json!({
        "grant_type": grant, "subject_token": "x", "subject_token_type": "urn:ietf:params:oauth:token-type:id_token" }), true).await;
    assert_eq!((s, p["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_request")), "only a refresh token is exchanged");
    let (s, p) = send(&app, "POST", "/api/v2/auth/token", json!({
        "grant_type": grant, "subject_token": "x", "subject_token_type": crate::v2::auth::TOKEN_TYPE_REFRESH,
        "requested_token_type": "urn:ietf:params:oauth:token-type:jwt" }), true).await;
    assert_eq!((s, p["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_request")));
}

#[tokio::test]
async fn the_code_grant_checks_pkce_and_the_redirect_before_asking_the_provider() {
    let google = json!({ "auth": { "google": { "enabled": true, "client_ids": "web-id",
        "desktop_client_id": "desktop-id", "desktop_client_secret": "desktop-secret" } } });
    let state = state(google).await;
    let app = app(&state, &Limiters::disabled());
    let verifier = "a".repeat(43);
    let code = |provider: &str, verifier: &str, redirect: &str| json!({
        "grant_type": "authorization_code", "provider": provider, "code": "c", "code_verifier": verifier, "redirect_uri": redirect });
    for (body, why) in [
        (code("google", "short", "http://127.0.0.1:5000/"), "a verifier under 43 characters"),
        (code("google", &format!("{}!", "a".repeat(43)), "http://127.0.0.1:5000/"), "a verifier outside RFC 7636's characters"),
        (code("google", &verifier, "https://evil.example/cb"), "a redirect that is not loopback"),
        (code("google", &verifier, "http://192.168.1.5:5000/"), "a LAN address"),
        (code("apple", &verifier, "http://127.0.0.1:5000/"), "a provider without a code flow"),
        (code("microsoft", &verifier, "http://127.0.0.1:5000/"), "a provider not set up here"),
        (json!({ "grant_type": "authorization_code", "provider": "google", "code": "c" }), "no verifier"),
    ] {
        let (s, p) = send(&app, "POST", "/api/v2/auth/token", body, true).await;
        assert_eq!((s, p["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_request")), "{why}");
    }
}

#[tokio::test]
async fn discovery_without_a_database_still_answers() {
    let google = json!({ "auth": { "google": { "enabled": true, "client_ids": "web-id",
        "desktop_client_id": "desktop-id", "desktop_client_secret": "", "web_client_id": "web-id" } } });
    let state = state(google).await;
    let app = axum::Router::new().route("/.well-known/own-audio", axum::routing::get(crate::v2::discovery::well_known)).with_state(state);
    let request = Request::builder().uri("/.well-known/own-audio").body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    let d: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(d["auth"]["clients"]["google"]["web"], "web-id");
    assert!(d["auth"]["clients"]["google"].get("desktop").is_none(), "a desktop client without its secret cannot exchange codes");
    let grants: Vec<&str> = d["auth"]["grants"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(grants.contains(&crate::v2::auth::GRANT_TOKEN_EXCHANGE) && grants.contains(&crate::v2::auth::GRANT_ASSERTION));
    assert!(!grants.contains(&"authorization_code"));
    assert_eq!((d["auth"]["password_reset"].as_bool(), d["auth"]["email_verification"].as_bool()), (Some(false), Some(false)));
    assert!(d.get("demo").is_none());
}
