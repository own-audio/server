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

// ── Phase 2: podcasts and the directory ─────────────────────────────────────

#[tokio::test]
async fn podcast_routes_need_a_signed_in_member() {
    let state = state(json!({})).await;
    let app = app(&state, &Limiters::disabled());
    let id = uuid::Uuid::new_v4();
    for (method, path) in [
        ("POST", "/api/v2/podcasts".to_string()),
        ("DELETE", format!("/api/v2/podcasts/{id}")),
        ("PATCH", format!("/api/v2/podcasts/{id}")),
        ("POST", format!("/api/v2/podcasts/{id}/refresh")),
        ("POST", "/api/v2/podcasts/refresh".to_string()),
        ("POST", format!("/api/v2/podcasts/{id}/store")),
        ("PUT", format!("/api/v2/episodes/{id}/stored")),
        ("DELETE", format!("/api/v2/episodes/{id}/stored")),
        ("GET", format!("/api/v2/episodes/{id}/transcript")),
        ("GET", "/api/v2/me/podcasts/unplayed".to_string()),
        ("GET", "/api/v2/podcasts/directory/search?q=news".to_string()),
        ("GET", "/api/v2/podcasts/directory/categories".to_string()),
        ("GET", "/api/v2/podcasts/directory?category=news".to_string()),
        ("GET", "/api/v2/podcasts/directory/similar?feed_url=https://example.com/f".to_string()),
        ("GET", "/api/v2/podcasts/directory/preview?feed_url=https://example.com/f".to_string()),
    ] {
        let (s, p) = send(&app, method, &path, json!({ "feed_url": "https://example.com/f" }), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthorized")), "{method} {path}");
    }
}

#[tokio::test]
async fn deleting_and_the_trash_need_a_signed_in_member() {
    let state = state(json!({})).await;
    let app = app(&state, &Limiters::disabled());
    let id = uuid::Uuid::new_v4();
    for (method, path) in [
        ("DELETE", format!("/api/v2/tracks/{id}")),
        ("DELETE", format!("/api/v2/books/{id}")),
        ("POST", "/api/v2/trash".to_string()),
        ("GET", "/api/v2/trash".to_string()),
        ("DELETE", "/api/v2/trash".to_string()),
        ("POST", format!("/api/v2/trash/batches/{id}/restore")),
        ("POST", format!("/api/v2/trash/track/{id}/restore")),
        ("DELETE", format!("/api/v2/trash/book/{id}")),
    ] {
        let (s, p) = send(&app, method, &path, json!({ "items": [{ "kind": "track", "id": id }] }), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthorized")), "{method} {path}");
    }
}

/// A stand-in for the metadata service's catalogue: an old one that reads a list of
/// languages as one language nobody publishes in, so answers only one language at a time.
async fn stub_catalogue() -> String {
    use axum::extract::Query;
    use axum::routing::get;
    use std::collections::HashMap;
    let entry = |id: i64, lang: &str| json!({ "id": id, "url": format!("https://example.com/{id}"), "title": format!("Show {id}"),
        "language": lang, "episode_count": 3, "categories": ["news"] });
    let by_language = move |q: HashMap<String, String>| -> Vec<Value> {
        match q.get("language").map(String::as_str) {
            Some("cs") => vec![entry(1, "cs"), entry(2, "cs")],
            Some("en") => vec![entry(3, "en"), entry(2, "cs")],
            Some(_) => vec![],
            None => vec![entry(9, "de")],
        }
    };
    let router = axum::Router::new()
        .route("/v1/podcasts/search", get(move |Query(q): Query<HashMap<String, String>>| async move { axum::Json(by_language(q)) }))
        .route("/v1/podcasts/browse", get(move |Query(q): Query<HashMap<String, String>>| async move {
            let offset: usize = q.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
            axum::Json(by_language(q).into_iter().skip(offset).collect::<Vec<_>>())
        }))
        .route("/v1/podcasts/categories", get(|| async { axum::Json(json!([{ "category": "news", "feed_count": 10, "active_count": 4 }])) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test]
async fn the_directory_asks_the_catalogue_in_the_listeners_languages() {
    let base_url = stub_catalogue().await;
    let config = crate::app::config::MetadataConfig { base_url, api_key: "k".into() };
    let languages = vec!["cs".to_string(), "en".to_string()];
    // Asked for `cs,en`, the old catalogue answers nothing; one language at a time, merged
    // round-robin and each show once.
    let found = crate::podcasts::search_catalogue(&config, "news", &languages, 25).await.unwrap();
    assert_eq!(found.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 3, 2]);
    let shows: Vec<crate::v2::podcasts::DirectoryShow> = found.into_iter().map(Into::into).collect();
    assert_eq!((shows[0].feed_url.as_str(), shows[0].episode_count, shows[0].podcast_id), ("https://example.com/1", Some(3), None));
    // No languages: no filter at all.
    let all = crate::podcasts::search_catalogue(&config, "news", &[], 25).await.unwrap();
    assert_eq!(all.iter().map(|e| e.id).collect::<Vec<_>>(), vec![9]);
    let browsed = crate::podcasts::browse_catalogue(&config, "news", &["cs".to_string()], 25, 1).await.unwrap();
    assert_eq!(browsed.iter().map(|e| e.id).collect::<Vec<_>>(), vec![2], "the offset is the cursor's");
    let categories = crate::metadata::mirror::MetadataMirror::new(&config).podcast_categories().await.unwrap();
    assert_eq!((categories[0].category.as_str(), categories[0].active_count), ("news", 4));
}

#[tokio::test]
async fn following_refreshing_and_previewing_share_v1s_outbound_bucket() {
    let state = state(json!({})).await;
    let id = uuid::Uuid::new_v4();
    for (method, path) in [
        ("POST", "/api/v2/podcasts".to_string()),
        ("POST", format!("/api/v2/podcasts/{id}/refresh")),
        ("POST", "/api/v2/podcasts/refresh".to_string()),
        ("GET", "/api/v2/podcasts/directory/preview?feed_url=https://example.com/f".to_string()),
    ] {
        let mut limits = Limiters::disabled();
        limits.outbound = crate::http::rate_limit::Limiter::per_minute(2, false);
        let app = app(&state, &limits);
        for _ in 0..2 {
            let (s, _) = send(&app, method, &path, json!({}), false).await;
            assert_ne!(s, StatusCode::TOO_MANY_REQUESTS, "{path} limited too early");
        }
        let (s, p) = send(&app, method, &path, json!({}), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{method} {path}");
        let (s, _) = send(&app, "GET", "/api/v2/podcasts", json!({}), false).await;
        assert_ne!(s, StatusCode::TOO_MANY_REQUESTS, "reading the list is not in the bucket");
    }
}

// ── Phase 5: books, series, collections, tags, contributors ─────────────────

#[tokio::test]
async fn book_shelf_and_contributor_routes_need_a_signed_in_member() {
    let state = state(json!({})).await;
    let app = app(&state, &Limiters::disabled());
    let id = uuid::Uuid::new_v4();
    for (method, path) in [
        ("PATCH", format!("/api/v2/books/{id}")),
        ("POST", format!("/api/v2/books/{id}/files")),
        ("PATCH", format!("/api/v2/books/{id}/files/{id}")),
        ("DELETE", format!("/api/v2/books/{id}/files/{id}")),
        ("PUT", format!("/api/v2/books/{id}/files/order")),
        ("POST", format!("/api/v2/books/{id}/identity/search")),
        ("PUT", format!("/api/v2/books/{id}/identity")),
        ("PUT", format!("/api/v2/books/{id}/image")),
        ("DELETE", format!("/api/v2/books/{id}/image")),
        ("PUT", format!("/api/v2/books/{id}/tags")),
        ("GET", "/api/v2/book-tags".to_string()),
        ("GET", "/api/v2/books?tag=x".to_string()),
        ("GET", "/api/v2/series".to_string()),
        ("POST", "/api/v2/series".to_string()),
        ("GET", format!("/api/v2/series/{id}")),
        ("PATCH", format!("/api/v2/series/{id}")),
        ("DELETE", format!("/api/v2/series/{id}")),
        ("PUT", format!("/api/v2/series/{id}/books")),
        ("GET", "/api/v2/collections".to_string()),
        ("POST", "/api/v2/collections".to_string()),
        ("GET", format!("/api/v2/collections/{id}")),
        ("PATCH", format!("/api/v2/collections/{id}")),
        ("DELETE", format!("/api/v2/collections/{id}")),
        ("GET", format!("/api/v2/collections/{id}/books")),
        ("POST", format!("/api/v2/collections/{id}/books")),
        ("DELETE", format!("/api/v2/collections/{id}/books/{id}")),
        ("POST", "/api/v2/contributors".to_string()),
        ("DELETE", format!("/api/v2/contributors/{id}")),
        ("PUT", format!("/api/v2/contributors/{id}/image")),
        ("DELETE", format!("/api/v2/contributors/{id}/image")),
    ] {
        let (s, p) = send(&app, method, &path, json!({ "name": "x", "title": "x", "tags": [], "books": [], "book_ids": [], "file_ids": [] }), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthorized")), "{method} {path}");
    }
}

#[tokio::test]
async fn identifying_a_book_shares_v1s_outbound_bucket() {
    let state = state(json!({})).await;
    let id = uuid::Uuid::new_v4();
    for path in [format!("/api/v2/books/{id}/identity/search"), format!("/api/v2/books/{id}/identity")] {
        let method = if path.ends_with("search") { "POST" } else { "PUT" };
        let mut limits = Limiters::disabled();
        limits.outbound = crate::http::rate_limit::Limiter::per_minute(2, false);
        let app = app(&state, &limits);
        for _ in 0..2 {
            let (s, _) = send(&app, method, &path, json!({}), false).await;
            assert_ne!(s, StatusCode::TOO_MANY_REQUESTS, "{path} limited too early");
        }
        let (s, p) = send(&app, method, &path, json!({}), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")), "{method} {path}");
    }
}

/// A stand-in for Open Library: one work found by title and author, and its description.
async fn stub_open_library() -> String {
    use axum::extract::Query;
    use axum::routing::get;
    use std::collections::HashMap;
    let router = axum::Router::new()
        .route("/search.json", get(|Query(q): Query<HashMap<String, String>>| async move {
            let found = q.get("title").map(String::as_str) == Some("Emma") || q.get("q").is_some_and(|k| k == "key:/works/OL66554W");
            let docs = if found {
                vec![json!({ "key": "/works/OL66554W", "title": "Emma", "author_name": ["Jane Austen"], "first_publish_year": 1815,
                    "cover_i": 9278312, "subject": ["Fiction"] })]
            } else {
                vec![]
            };
            axum::Json(json!({ "docs": docs }))
        }))
        .route("/works/OL66554W.json", get(|| async { axum::Json(json!({ "description": { "type": "/type/text", "value": "A matchmaker." } })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test]
async fn a_book_identity_search_reads_the_source_and_answers_items() {
    let base = stub_open_library().await;
    let items = crate::metadata::open_library::search_at(&base, Some("Emma"), Some("Jane Austen"), 10).await.unwrap();
    let answer = serde_json::to_value(crate::v2::books::BookCandidates { items: items.into_iter().map(Into::into).collect() }).unwrap();
    let first = &answer["items"][0];
    assert_eq!(first["volume_id"], "ol:OL66554W");
    assert_eq!(first["source"], "openlibrary");
    assert_eq!((first["title"].as_str(), first["author"].as_str()), (Some("Emma"), Some("Jane Austen")));
    assert_eq!(first["description"], "A matchmaker.", "the first hits are described");
    assert_eq!(first["published_year"], 1815);
    // Nothing for the words: an empty list, not an error.
    let none = crate::metadata::open_library::search_at(&base, Some("Nothing Like It"), None, 10).await.unwrap();
    assert!(none.is_empty());
    // Keeping a match fetches the work again by its id.
    let work = crate::metadata::open_library::work_at(&base, "OL66554W").await.unwrap();
    assert_eq!((work.volume_id.as_str(), work.description.as_deref()), ("ol:OL66554W", Some("A matchmaker.")));
}

#[tokio::test]
async fn track_playlist_and_feedback_routes_need_a_signed_in_member() {
    let state = state(json!({})).await;
    let app = app(&state, &Limiters::disabled());
    let id = uuid::Uuid::new_v4();
    for (method, path) in [
        ("PUT", format!("/api/v2/tracks/{id}/lyrics")),
        ("GET", format!("/api/v2/tracks/{id}/file-tags")),
        ("POST", format!("/api/v2/tracks/{id}/rescan")),
        ("GET", "/api/v2/tracks/duplicates".to_string()),
        ("GET", "/api/v2/tracks/duplicates?include=tracks".to_string()),
        ("PUT", format!("/api/v2/tracks/{id}/image")),
        ("DELETE", format!("/api/v2/tracks/{id}/image")),
        ("PUT", format!("/api/v2/playlists/{id}/image")),
        ("DELETE", format!("/api/v2/playlists/{id}/image")),
        ("PATCH", format!("/api/v2/playlists/{id}")),
        ("GET", "/api/v2/me/feedback".to_string()),
        ("PUT", format!("/api/v2/me/feedback/track/{id}")),
        ("DELETE", format!("/api/v2/me/feedback/track/{id}")),
        ("POST", "/api/v2/smart-playlists/intent".to_string()),
    ] {
        let (s, p) = send(&app, method, &path, json!({ "text": "jazz", "value": "ban", "upload_id": "x", "kept": true }), false).await;
        assert_eq!((s, p["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthorized")), "{method} {path}");
    }
}
