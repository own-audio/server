// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/api/v2` (docs/api-v2-plan.md; own-audio/api design/DESIGN.md). Its own
//! router and OpenAPI document beside v1, on the entities of
//! docs/data-model-plan.md. Errors are problem details ([`problem`]), lists
//! are cursor pages ([`page`]), and every operation declares its permission as
//! `x-permission` (security hardening plan §5.3).

pub mod discovery;
pub mod me;
pub mod page;
pub mod problem;

use crate::app::AppState;
use crate::http::rate_limit::Limiters;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::{router::OpenApiRouter, routes};

/// What an operation may declare as `x-permission`. The extractors enforce it;
/// the conformance `access` suite checks the answers against it.
pub const PERMISSIONS: &[&str] =
    &["public", "user", "member", "family_admin", "instance_admin", "can_upload", "can_generate", "owner"];

/// The routes under `/api/v2`, documented.
pub fn router(_limits: &Limiters) -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(me::me))
}

/// `/api/v2` as served: the per-caller budget on every route, unknown paths a
/// `404` problem, and every error answer in problem shape.
pub fn mount(limits: &Limiters) -> axum::Router<AppState> {
    let api: axum::Router<AppState> = router(limits).into();
    limits
        .api
        .wrap(api)
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
        for op in [&mut item.get, &mut item.put, &mut item.post, &mut item.delete, &mut item.patch].into_iter().flatten() {
            crate::http::openapi::tidy(op);
            let public = permission(op) == Some("public");
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

fn permission(op: &utoipa::openapi::path::Operation) -> Option<&str> {
    op.extensions.as_ref()?.get("x-permission")?.as_str()
}

#[cfg(test)]
mod tests {
    const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/api/openapi-v2.json");

    #[test]
    fn openapi_v2_json_is_current() {
        let generated = super::document().to_pretty_json().unwrap() + "\n";
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
