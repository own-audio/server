// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/api/v2` (docs/api-v2-plan.md; own-audio/api design/DESIGN.md). Its own
//! router and OpenAPI document beside v1, on the entities of
//! docs/data-model-plan.md. Errors are problem details ([`problem`]), lists
//! are cursor pages ([`page`]), and every operation declares its permission as
//! `x-permission` (security hardening plan §5.3).

pub mod auth;
pub mod discovery;
pub mod edit;
pub mod events;
pub mod household;
pub mod idempotency;
pub mod images;
pub mod me;
pub mod media;
pub mod music;
pub mod page;
pub mod problem;
pub mod spoken;
pub mod state;
pub mod sync;
pub mod uploads;

use crate::app::AppState;
use crate::http::rate_limit::Limiters;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::{router::OpenApiRouter, routes};

/// What an operation may declare as `x-permission`. The extractors enforce it;
/// the conformance `access` suite checks the answers against it.
pub const PERMISSIONS: &[&str] =
    &["public", "user", "member", "family_admin", "instance_admin", "can_upload", "can_generate", "owner"];

/// The routes under `/api/v2`, documented. Sign-in shares v1's stricter
/// buckets: a password guess costs the same on either version.
pub fn router(limits: &Limiters) -> OpenApiRouter<AppState> {
    use crate::http::openapi::map;
    OpenApiRouter::new()
        .routes(map(routes!(auth::token), |m| limits.login.apply(m)))
        .routes(routes!(auth::revoke))
        .routes(map(routes!(auth::device_authorization), |m| limits.device.apply(m)))
        .routes(routes!(auth::describe_device))
        .routes(routes!(auth::approve_device))
        .routes(routes!(auth::deny_device))
        .routes(routes!(me::me))
        .routes(routes!(me::sessions, me::delete_sessions))
        .routes(routes!(me::delete_session))
        .routes(routes!(me::tokens, me::create_token))
        .routes(routes!(me::delete_token))
        .routes(routes!(music::list_contributors))
        .routes(routes!(music::get_contributor, edit::patch_contributor))
        .routes(routes!(music::list_artists))
        .routes(routes!(music::list_authors))
        .routes(routes!(music::list_narrators))
        .routes(routes!(music::list_hosts))
        .routes(routes!(music::list_albums))
        .routes(routes!(music::get_album, edit::patch_album))
        .routes(routes!(music::get_release_group))
        .routes(routes!(music::get_recording))
        .routes(routes!(music::list_genres))
        .routes(routes!(music::list_tracks, uploads::create_track))
        .routes(routes!(music::get_track))
        .routes(routes!(spoken::list_books, uploads::create_book))
        .routes(routes!(spoken::get_book))
        .routes(routes!(spoken::list_podcasts))
        .routes(routes!(spoken::get_podcast))
        .routes(routes!(spoken::list_episodes))
        .routes(routes!(spoken::get_episode))
        .routes(routes!(edit::set_track_credits))
        .routes(routes!(edit::set_book_credits))
        .routes(routes!(edit::set_podcast_credits))
        .routes(routes!(edit::set_album_credits))
        .routes(routes!(edit::merge_contributor))
        .routes(routes!(edit::split_contributor))
        .routes(routes!(edit::set_release_group))
        .routes(routes!(edit::merge_album))
        .routes(routes!(state::list_favourites))
        .routes(routes!(state::put_favourite, state::delete_favourite))
        .routes(routes!(state::list_ratings))
        .routes(routes!(state::put_rating))
        .routes(routes!(state::list_progress))
        .routes(routes!(state::get_progress, state::put_progress, state::delete_progress))
        .routes(routes!(state::post_plays))
        .routes(routes!(state::list_bookmarks, state::create_bookmark))
        .routes(routes!(state::patch_bookmark, state::delete_bookmark))
        .routes(routes!(state::push))
        .routes(routes!(sync::changes))
        .routes(routes!(sync::snapshot))
        .routes(routes!(media::playback_info))
        .routes(routes!(media::stream))
        .routes(routes!(images::image))
        .routes(routes!(household::get_family, household::patch_family))
        .routes(routes!(household::list_members))
        .routes(routes!(household::patch_member, household::remove_member))
        .routes(routes!(household::list_invites, household::create_invite))
        .routes(routes!(household::delete_invite))
        .routes(routes!(household::join))
        .routes(routes!(household::storage))
        .routes(routes!(household::set_track_visibility))
        .routes(routes!(household::set_book_visibility))
        .routes(routes!(household::set_podcast_visibility))
        .routes(routes!(events::events))
        .routes(routes!(uploads::create_upload))
}

/// The scope a personal access token needs for an operation, from the v2
/// document's `x-scope`; `None` for a path that is not a v2 operation.
pub fn required_scope(method: &str, path: &str) -> Option<&'static str> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static TABLE: OnceLock<HashMap<(String, String), &'static str>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        for (path, item) in document().paths.paths {
            for (method, op) in [("GET", &item.get), ("PUT", &item.put), ("POST", &item.post), ("DELETE", &item.delete), ("PATCH", &item.patch)] {
                let scope = op.as_ref().and_then(|op| op.extensions.as_ref()?.get("x-scope")?.as_str().map(str::to_string));
                if let Some(scope) = scope.and_then(|s| auth::SCOPES.iter().find(|k| **k == s)) {
                    table.insert((method.to_string(), path.clone()), *scope);
                }
            }
        }
        table
    });
    let path = if path.starts_with("/api/v2") { path.to_string() } else { format!("/api/v2{path}") };
    table.get(&(method.to_ascii_uppercase(), path)).copied()
}

/// `/api/v2` as served: the per-caller budget on every route, `Idempotency-Key`
/// on writes, unknown paths a `404` problem, and every error answer in problem
/// shape.
pub fn mount(limits: &Limiters, state: &AppState, edition: axum::Router<AppState>) -> axum::Router<AppState> {
    let api: axum::Router<AppState> = axum::Router::from(router(limits)).merge(edition);
    limits
        .api
        .wrap(api)
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), idempotency::middleware))
        .fallback(problem::fallback)
        .layer(axum::middleware::from_fn(problem::middleware))
}

struct Security;

impl Modify for Security {
    fn modify(&self, api: &mut utoipa::openapi::OpenApi) {
        let components = api.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .description(Some("An access token from `POST /api/v2/auth/token`."))
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "own.audio API v2",
        description = "The own.audio API (own-audio/api design/DESIGN.md). Draft until our apps ship on it: it may change in any way until then.",
        license(name = "AGPL-3.0-or-later", identifier = "AGPL-3.0-or-later"),
    ),
    servers((url = "/", description = "This server")),
    components(schemas(problem::ProblemBody, problem::FieldError)),
    modifiers(&Security),
)]
struct V2Doc;

/// The v2 document, as `docs/api/openapi-v2.json` holds it. Every error answer
/// is problem details; every signed-in operation can answer `401`, and every
/// operation `429`, so those are stated once here rather than on each handler.
pub fn document() -> utoipa::openapi::OpenApi {
    use utoipa::openapi::{ContentBuilder, RefOr, Ref, ResponseBuilder};
    let limits = Limiters::from_config(&crate::app::config::RateLimitConfig::default());
    let (_, mut api) = OpenApiRouter::<AppState>::with_openapi(V2Doc::openapi())
        .routes(routes!(discovery::well_known))
        .nest("/api/v2", router(&limits))
        .split_for_parts();
    api.info.version = format!("2.{}-{}", discovery::V2_REVISION, discovery::V2_STATUS);
    let problem = || {
        ContentBuilder::new().schema(Some(RefOr::Ref(Ref::from_schema_name("ProblemBody")))).build()
    };
    let error = |description: &str| {
        ResponseBuilder::new().description(description).content(problem::CONTENT_TYPE, problem()).build()
    };
    for item in api.paths.paths.values_mut() {
        for (read, op) in [(true, &mut item.get), (false, &mut item.put), (false, &mut item.post), (false, &mut item.delete), (false, &mut item.patch)]
            .into_iter()
            .filter_map(|(read, op)| op.as_mut().map(|op| (read, op)))
        {
            crate::http::openapi::tidy(op);
            let public = permission(op) == Some("public");
            // What a personal access token needs: stated, or by what the operation is.
            if !public && op.extensions.as_ref().and_then(|e| e.get("x-scope")).is_none() {
                let scope = match permission(op) {
                    Some("instance_admin") => "admin",
                    Some("family_admin") => "family:admin",
                    _ if read => "library:read",
                    _ => "library:write",
                };
                let mut extensions = op.extensions.take().unwrap_or_default();
                extensions.merge(utoipa::openapi::extensions::ExtensionsBuilder::new().add("x-scope", scope).build());
                op.extensions = Some(extensions);
            }
            if public {
                op.security = Some(vec![utoipa::openapi::security::SecurityRequirement::default()]);
            } else {
                op.responses
                    .responses
                    .entry("401".into())
                    .or_insert_with(|| RefOr::T(error("Missing, invalid or revoked access token")));
            }
            op.responses.responses.entry("429".into()).or_insert_with(|| RefOr::T(error("Rate limited; see `Retry-After`")));
            for (status, response) in op.responses.responses.iter_mut() {
                if status.starts_with('4') || status.starts_with('5') {
                    if let RefOr::T(r) = response {
                        r.content.clear();
                        r.content.insert(problem::CONTENT_TYPE.into(), problem());
                    }
                }
            }
        }
    }
    api
}

/// The document as committed: utoipa keeps extensions in a hash map, so their
/// order changes from run to run; here each operation's `x-` members come last,
/// sorted.
pub fn document_json() -> String {
    let mut value = serde_json::to_value(document()).expect("the v2 document serialises");
    if let Some(paths) = value.get_mut("paths").and_then(|p| p.as_object_mut()) {
        for item in paths.values_mut().filter_map(|i| i.as_object_mut()) {
            for op in item.values_mut().filter_map(|o| o.as_object_mut()) {
                let mut ext: Vec<(String, serde_json::Value)> = Vec::new();
                for key in op.keys().filter(|k| k.starts_with("x-")).cloned().collect::<Vec<_>>() {
                    if let Some(v) = op.remove(&key) {
                        ext.push((key, v));
                    }
                }
                ext.sort_by(|a, b| a.0.cmp(&b.0));
                op.extend(ext);
            }
        }
    }
    serde_json::to_string_pretty(&value).expect("the v2 document serialises") + "\n"
}

fn permission(op: &utoipa::openapi::path::Operation) -> Option<&str> {
    op.extensions.as_ref()?.get("x-permission")?.as_str()
}

#[cfg(test)]
mod tests {
    const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/api/openapi-v2.json");

    #[test]
    fn openapi_v2_json_is_current() {
        let generated = super::document_json();
        if std::env::var_os("OPENAPI_WRITE").is_some() {
            std::fs::write(FILE, &generated).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(FILE).unwrap_or_default();
        assert!(
            committed == generated,
            "docs/api/openapi-v2.json is out of date: run `OPENAPI_WRITE=1 cargo test openapi` and commit it"
        );
    }

    /// Deny by default: an operation that does not say who may call it does not ship.
    #[test]
    fn openapi_v2_every_operation_declares_its_permission() {
        let api = super::document();
        let mut missing = Vec::new();
        for (path, item) in &api.paths.paths {
            for (method, op) in [("get", &item.get), ("put", &item.put), ("post", &item.post), ("delete", &item.delete), ("patch", &item.patch)] {
                let Some(op) = op else { continue };
                match super::permission(op) {
                    Some(p) if super::PERMISSIONS.contains(&p) => {}
                    other => missing.push(format!("{method} {path}: {other:?}")),
                }
            }
        }
        assert!(missing.is_empty(), "operations without a valid x-permission: {missing:#?}");
    }
}
