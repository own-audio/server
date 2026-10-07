// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::app::AppState;
use crate::http::handlers;
use axum::Router;
use utoipa_axum::{router::OpenApiRouter, routes};
use axum::routing::get;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

/// Core API routes (without state applied).
///
/// Returns `Router<AppState>` so callers (e.g. the SaaS binary) can
/// merge additional routes before calling [`finalize`].
pub fn api_routes(config: &crate::app::AppConfig) -> Router<AppState> {
    let limits = crate::http::rate_limit::Limiters::from_config(&config.server.rate_limit);
    api_router(&limits).into()
}

/// The same routes with their OpenAPI description (`http::openapi`). A route
/// registered through `routes!` is documented by construction; one added with
/// plain `.route` is not, and `openapi_covers_every_route` lists it.
pub fn api_router(limits: &crate::http::rate_limit::Limiters) -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(crate::http::server_info::server_info))
        // Signed links instead of presigned store URLs: local storage, or S3
        // behind STORAGE__PROXY. The signature is the authorisation.
        .routes(crate::http::openapi::map(routes!(crate::http::media::media), |m| {
            m.layer(axum::extract::DefaultBodyLimit::disable())
        }))
        .nest("/setup", crate::setup::router(limits))
        .nest("/auth", crate::auth::router(limits))
        .nest("/users", crate::users::router())
        .nest("/family", crate::families::router())
        .nest("/trash", crate::trash::router())
        .nest("/sync", crate::filesync::router())
        // Instance-admin only, cross-family — every family on the server,
        // not the caller's own. See families::admin's own doc comment.
        .nest("/admin/families", crate::families::admin::router())
        // Cross-domain Home dashboard — counts and activity for the whole
        // server. See dashboard's own doc comment.
        .nest("/admin/stats", crate::dashboard::router())
        // Public — the QR/link landing page and account-claim flow. No
        // FamilyContext/AuthUser extractor on these handlers, matching
        // /auth/register and /auth/login's own unauthenticated routes.
        .nest("/join", crate::families::join_router(limits))
        .nest("/library", crate::library::router())
        .nest("/podcasts", crate::podcasts::router())
        .nest("/audiobooks", crate::audiobooks::router().into())
        .nest("/music", crate::music::router().into())
        .nest("/playback", crate::playback::router())
        .nest("/jobs", crate::jobs::router())
        .nest("/uploads", crate::uploads::router())
        .nest("/stats", crate::stats::router())
        .nest("/devices", crate::devices::router())
}

/// Paths under `/api/v1` that belong to the hosted edition, and the
/// `features` key each one is reported under. A core-only build answers them
/// with `501 feature_unavailable` instead of `404 not_found`, so an old or
/// unaware client gets a stable, actionable error (API_COMPATIBILITY.md §6).
/// `billing::routes()` is the other half of this list.
pub fn hosted_feature_for(path: &str) -> Option<&'static str> {
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let admin_credit = p.starts_with("/admin/families/") && p.trim_end_matches('/').ends_with("/credit");
    if p.starts_with("/billing") || p.starts_with("/family/billing") || admin_credit {
        Some("billing")
    } else if p.starts_with("/audiobook-gen") {
        Some("narration")
    } else if p.starts_with("/podcast-translate") {
        Some("translation")
    } else {
        None
    }
}

/// Wrap an API router with middleware, health endpoint, SPA fallback, and state.
///
/// `root` is merged at the site root for an edition's non-API pages (the
/// hosted edition's Stripe landing pages); the core passes an empty router.
pub fn finalize(api: Router<AppState>, root: Router<AppState>, state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        // The API answers 404 (or 501 for hosted-only paths) for its own unknown
        // paths. Without this the outer SPA fallback below would hand an API
        // client a page of HTML with status 200.
        .nest("/api/v1", api.fallback(handlers::api_fallback))
        .merge(root)
        // OpenSubsonic-compatible surface for external music clients. A
        // sibling of /api/v1 (not nested under it) because Subsonic clients
        // call fixed paths like /rest/ping.view.
        .nest(
            "/rest",
            crate::subsonic::router()
                .layer(axum::middleware::from_fn(
                    crate::subsonic::form::merge_post_form,
                ))
                .fallback(handlers::not_found),
        )
        // A real SPA fallback: unknown paths get index.html so the console's own router
        // can render /join/<code>, /audiobooks/<id> and friends. ServeDir alone 404s
        // them, because they are routes rather than files on disk — which is why every
        // deep link into a self-hosted console was broken.
        .fallback_service(
            ServeDir::new("ui/dist").not_found_service(ServeFile::new("ui/dist/index.html")),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Assemble the full core-only Axum router.
pub fn build(state: AppState) -> Router {
    let api = api_routes(state.config());
    finalize(api, Router::new(), state)
}

#[cfg(test)]
mod tests {
    use super::hosted_feature_for;

    #[test]
    fn hosted_paths_map_to_their_feature() {
        assert_eq!(hosted_feature_for("/api/v1/family/billing"), Some("billing"));
        assert_eq!(hosted_feature_for("/api/v1/family/billing/topup"), Some("billing"));
        assert_eq!(hosted_feature_for("/api/v1/billing/stripe/webhook"), Some("billing"));
        assert_eq!(hosted_feature_for("/api/v1/admin/families/abc/credit"), Some("billing"));
        assert_eq!(hosted_feature_for("/api/v1/audiobook-gen/jobs"), Some("narration"));
        assert_eq!(hosted_feature_for("/api/v1/podcast-translate/episodes/x"), Some("translation"));
        assert_eq!(hosted_feature_for("/api/v1/family"), None);
        assert_eq!(hosted_feature_for("/api/v1/admin/families/abc"), None);
        assert_eq!(hosted_feature_for("/api/v1/nope"), None);
    }
}

