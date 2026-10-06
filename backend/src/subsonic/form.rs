// SPDX-License-Identifier: AGPL-3.0-or-later
/// Accepts POSTed form parameters by folding them into the query string.
///
/// OpenSubsonic's `formPost` extension lets a client send the same parameters
/// as a `application/x-www-form-urlencoded` body instead of a query string.
/// Clients reach for it when a request would otherwise be too long for a URL —
/// `savePlayQueue` with a few hundred track ids is the usual case, and it is
/// exactly the request most likely to be silently truncated by a proxy.
///
/// Rather than teach every extractor to look in two places, this rewrites the
/// request before routing: the body is appended to the URI's query and then
/// dropped. Auth, `SubsonicQuery`, and every handler keep reading query
/// parameters and never learn the difference.
use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::Response;

/// Enough for a very long play queue and nothing like enough to be worth
/// abusing — a few hundred UUIDs is roughly 15KB.
const MAX_FORM_BODY: usize = 64 * 1024;

pub async fn merge_post_form(request: Request, next: Next) -> Response {
    if request.method() != Method::POST || !is_form_encoded(&request) {
        return next.run(request).await;
    }

    let (mut parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_FORM_BODY).await {
        Ok(bytes) => bytes,
        // A body over the cap is refused here rather than truncated: half a
        // play queue silently saved is worse than a failed save.
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response_with_empty_body(),
    };

    if let Some(uri) = uri_with_form_params(&parts.uri, &bytes) {
        parts.uri = uri;
    }

    next.run(Request::from_parts(parts, Body::empty())).await
}

fn is_form_encoded(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        // The charset parameter is common and must not defeat the match.
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|base| base.trim().eq_ignore_ascii_case("application/x-www-form-urlencoded"))
        })
}

/// Appends an urlencoded body to the URI's existing query.
///
/// The body goes *after* any query parameters so that a client sending both
/// has the body win for repeated keys — but the two are concatenated rather
/// than merged, since Subsonic parameters are legitimately repeatable and
/// dropping duplicates would corrupt a play queue.
fn uri_with_form_params(uri: &Uri, body: &Bytes) -> Option<Uri> {
    let body = std::str::from_utf8(body).ok()?.trim();
    if body.is_empty() {
        return None;
    }

    let path = uri.path();
    let query = match uri.query().filter(|q| !q.is_empty()) {
        Some(existing) => format!("{existing}&{body}"),
        None => body.to_string(),
    };
    format!("{path}?{query}").parse().ok()
}

/// `StatusCode` alone has no body; this keeps the refusal a one-liner above.
trait EmptyBodyResponse {
    fn into_response_with_empty_body(self) -> Response;
}

impl EmptyBodyResponse for StatusCode {
    fn into_response_with_empty_body(self) -> Response {
        Response::builder().status(self).body(Body::empty()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_a_form_body_into_an_empty_query() {
        let uri: Uri = "/rest/ping.view".parse().unwrap();
        let merged = uri_with_form_params(&uri, &Bytes::from("u=someone&f=json")).unwrap();
        assert_eq!(merged.query(), Some("u=someone&f=json"));
        assert_eq!(merged.path(), "/rest/ping.view");
    }

    #[test]
    fn appends_to_an_existing_query_rather_than_replacing_it() {
        let uri: Uri = "/rest/star.view?u=someone".parse().unwrap();
        let merged = uri_with_form_params(&uri, &Bytes::from("id=1&id=2")).unwrap();
        assert_eq!(merged.query(), Some("u=someone&id=1&id=2"));
    }

    /// Repeated keys are the whole point of `formPost` for this API — a play
    /// queue is a list of `id` values and must survive the fold intact.
    #[test]
    fn keeps_every_repetition_of_a_key() {
        let uri: Uri = "/rest/savePlayQueue.view".parse().unwrap();
        let merged = uri_with_form_params(&uri, &Bytes::from("id=a&id=b&id=c")).unwrap();
        assert_eq!(merged.query().unwrap().matches("id=").count(), 3);
    }

    #[test]
    fn an_empty_body_leaves_the_uri_alone() {
        let uri: Uri = "/rest/ping.view?u=someone".parse().unwrap();
        assert!(uri_with_form_params(&uri, &Bytes::from("")).is_none());
    }
}
