// SPDX-License-Identifier: AGPL-3.0-or-later
//! The address a request came in on, when it is one of the server's own
//! (`SERVER__BASE_URL` or `SERVER__ADDRESSES`), so the links the server hands
//! out — media, uploads, images, streams — point back at that address.
//!
//! A server reachable at home and through a tunnel used to sign every link for
//! `SERVER__BASE_URL`: the console opened at the home address then uploaded to
//! the tunnel's — another origin, refused by CORS, and over the tunnel's
//! request-size limit besides. Links for the address in use keep each client on
//! the way it already reaches the server.
//!
//! Only a configured address is ever used: a `Host` header is the client's to
//! write, so anything else leaves the links on `SERVER__BASE_URL` as before.

use axum::extract::{Request, State};
use axum::http::header::HOST;
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

tokio::task_local! {
    static REQUEST_BASE: String;
}

/// The server's addresses as `(authority, origin)`: `authority` as a `Host`
/// header carries it (lowercase, default port left out), `origin` as given.
#[derive(Clone, Default)]
pub struct KnownAddresses(Arc<Vec<(String, String)>>);

impl KnownAddresses {
    pub fn new(base_url: Option<&str>, extra: Option<&str>) -> Self {
        let mut out: Vec<(String, String)> = Vec::new();
        for raw in base_url.into_iter().chain(extra.unwrap_or_default().split(',')) {
            let origin = raw.trim().trim_end_matches('/');
            if let Some(authority) = authority(origin) {
                if !out.iter().any(|(a, _)| *a == authority) {
                    out.push((authority, origin.to_string()));
                }
            }
        }
        Self(Arc::new(out))
    }

    /// The configured origin a `Host` header names, if any.
    pub fn matching(&self, host: &str) -> Option<&str> {
        let host = host.trim().to_ascii_lowercase();
        self.0.iter().find(|(a, _)| *a == host).map(|(_, o)| o.as_str())
    }
}

/// `host[:port]` of an origin, as a client puts it in `Host`: lowercase, and
/// without the scheme's default port.
fn authority(origin: &str) -> Option<String> {
    let (scheme, rest) = origin.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    if authority.is_empty() {
        return None;
    }
    let default = match scheme.to_ascii_lowercase().as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    Some(authority.strip_suffix(default).unwrap_or(&authority).to_string())
}

/// Runs the request with its address remembered, when it is one of ours.
pub async fn middleware(State(known): State<KnownAddresses>, request: Request, next: Next) -> Response {
    let matched = request
        .headers()
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| known.matching(h))
        .map(str::to_string);
    match matched {
        Some(base) => REQUEST_BASE.scope(base, next.run(request)).await,
        None => next.run(request).await,
    }
}

/// The base for a link: the address this request came in on, if it is one of
/// the server's, else `configured` (`SERVER__BASE_URL`). Outside a request — a
/// job, a stream body read after its handler returned — always `configured`.
pub fn base_or(configured: &str) -> String {
    REQUEST_BASE
        .try_with(|b| b.clone())
        .unwrap_or_else(|_| configured.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> KnownAddresses {
        KnownAddresses::new(Some("https://own-audio.example.eu/"), Some("http://192.168.88.97:8099, https://Other.example:8443"))
    }

    #[test]
    fn a_host_header_finds_its_configured_address() {
        let k = known();
        assert_eq!(k.matching("own-audio.example.eu"), Some("https://own-audio.example.eu"));
        assert_eq!(k.matching("192.168.88.97:8099"), Some("http://192.168.88.97:8099"));
        assert_eq!(k.matching("other.example:8443"), Some("https://Other.example:8443"));
        // The default port may be written or not.
        assert_eq!(KnownAddresses::new(Some("https://a.example"), None).matching("a.example"), Some("https://a.example"));
        assert_eq!(KnownAddresses::new(Some("https://a.example:443"), None).matching("a.example"), Some("https://a.example:443"));
    }

    #[test]
    fn anything_else_is_not_ours() {
        let k = known();
        assert_eq!(k.matching("evil.example"), None);
        assert_eq!(k.matching("192.168.88.97"), None, "another port is another address");
        assert_eq!(k.matching(""), None);
        assert_eq!(KnownAddresses::new(None, None).matching("localhost"), None);
    }

    #[tokio::test]
    async fn links_follow_the_request_and_fall_back_outside_it() {
        assert_eq!(base_or("https://configured.example/"), "https://configured.example");
        let inside = REQUEST_BASE.scope("http://192.168.88.97:8099".to_string(), async { base_or("https://configured.example") }).await;
        assert_eq!(inside, "http://192.168.88.97:8099");
    }
}
