// SPDX-License-Identifier: AGPL-3.0-or-later
//! The `/api/v1` contract as OpenAPI 3.1, generated from the handlers'
//! `#[utoipa::path]` annotations and committed as `docs/api/openapi.json`.
//!
//! The test below fails when the committed file is not what the code
//! describes; `OPENAPI_WRITE=1 cargo test openapi` rewrites it.
//! `info.version` is `1.<contract revision>`, so a release does not change
//! the file unless the contract did (`docs/API_COMPATIBILITY.md`).

use crate::http::server_info::{API_REVISION, API_VERSION};
use serde::Serialize;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;

/// Every error answer: a status code and this body.
#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    /// What went wrong, in English; not meant to be matched on.
    pub error: String,
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
                    .description(Some("The access token from `POST /auth/login` or `POST /auth/refresh`."))
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "own.audio server API",
        description = "The contract every own.audio client speaks. Additive within v1: see docs/API_COMPATIBILITY.md.",
        license(name = "AGPL-3.0-or-later", identifier = "AGPL-3.0-or-later"),
    ),
    components(schemas(ErrorBody)),
    modifiers(&Security),
)]
struct ApiDoc;

/// Wrap the method router `routes!` made, e.g. in a rate limit, keeping its
/// documentation: `.routes(openapi::map(routes!(login), |m| limits.login.apply(m)))`.
pub fn map<S>(
    (schemas, paths, method_router): utoipa_axum::router::UtoipaMethodRouter<S>,
    f: impl FnOnce(axum::routing::MethodRouter<S>) -> axum::routing::MethodRouter<S>,
) -> utoipa_axum::router::UtoipaMethodRouter<S> {
    (schemas, paths, f(method_router))
}

/// The whole document, as the committed file holds it.
pub fn document() -> utoipa::openapi::OpenApi {
    let limits = crate::http::rate_limit::Limiters::from_config(&crate::app::config::RateLimitConfig::default());
    let (_, mut api) = OpenApiRouter::<crate::app::AppState>::with_openapi(ApiDoc::openapi())
        .nest("/api/v1", crate::http::router::api_router(&limits))
        .split_for_parts();
    api.info.version = format!("{API_VERSION}.{API_REVISION}");
    for item in api.paths.paths.values_mut() {
        for op in [&mut item.get, &mut item.put, &mut item.post, &mut item.delete, &mut item.patch, &mut item.head]
            .into_iter()
            .flatten()
        {
            tidy(op);
        }
    }
    api
}

/// Handler doc comments start with `GET /api/v1/…` for readers of the code;
/// in the document the method and path are already there. The first
/// paragraph becomes the summary, the rest the description. Operation ids
/// get their tag in front, since handler names repeat across modules.
fn tidy(op: &mut utoipa::openapi::path::Operation) {
    let text = [op.summary.take(), op.description.take()].into_iter().flatten().collect::<Vec<_>>().join("\n\n");
    let text = text.trim_start();
    let text = match text.split_once(' ') {
        Some((method, rest))
            if ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&method) && rest.starts_with('/') =>
        {
            let rest = rest.split_once(|c: char| c.is_whitespace()).map_or("", |(_, r)| r);
            rest.trim_start().trim_start_matches(['—', '-', ':']).trim_start()
        }
        _ => text,
    };
    // The first sentence is the summary; everything is the description.
    let flat = text.split("\n\n").next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    let first = flat.split_once(". ").map_or(flat.as_str(), |(s, _)| s).trim_end_matches('.');
    let mut chars = first.chars();
    let summary = chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default();
    op.summary = (!summary.is_empty()).then_some(summary);
    let one_sentence = !text.contains("\n\n") && flat.trim_end_matches('.') == first;
    op.description = (!one_sentence).then(|| text.trim().to_string());
    if let (Some(id), Some(tag)) = (op.operation_id.as_mut(), op.tags.as_ref().and_then(|t| t.first())) {
        *id = format!("{tag}_{id}");
    }
}

#[cfg(test)]
mod tests {
    const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/api/openapi.json");

    #[test]
    fn openapi_json_is_current() {
        let generated = super::document().to_pretty_json().unwrap() + "\n";
        // OPENAPI_OUT writes somewhere else, to look at without touching the committed file.
        if let Some(out) = std::env::var_os("OPENAPI_OUT") {
            std::fs::write(out, &generated).unwrap();
            return;
        }
        if std::env::var_os("OPENAPI_WRITE").is_some() {
            std::fs::create_dir_all(std::path::Path::new(FILE).parent().unwrap()).unwrap();
            std::fs::write(FILE, &generated).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(FILE).unwrap_or_default();
        assert!(
            committed == generated,
            "docs/api/openapi.json is out of date: run `OPENAPI_WRITE=1 cargo test openapi` and commit it"
        );
    }
}
