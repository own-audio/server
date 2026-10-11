// SPDX-License-Identifier: AGPL-3.0-or-later
//! Browser-facing response headers (security hardening plan §7.1). Every
//! response carries them — API answers too, where they cost nothing — and
//! the console's are the ones that matter: a content security policy that
//! allows the console's own files, the two sign-in providers' scripts, and
//! media from wherever this install keeps it; no framing; no MIME sniffing;
//! a strict referrer; the camera only for the console's own QR scanner.
//!
//! `Strict-Transport-Security` goes out only when `SERVER__BASE_URL` is
//! https: a browser ignores it over plain http anyway, and an install that
//! moves back from https to http must not find its console unreachable for
//! a year.
use crate::app::AppConfig;
use axum::extract::Request;
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use url::Url;

#[derive(Clone)]
pub struct SecurityHeaders {
    csp: HeaderValue,
    hsts: Option<HeaderValue>,
}

impl SecurityHeaders {
    pub fn from_config(cfg: &AppConfig) -> Self {
        let csp = HeaderValue::from_str(&content_security_policy(cfg)).expect("a CSP without control characters");
        let hsts = cfg
            .server
            .base_url
            .as_deref()
            .filter(|b| b.starts_with("https://"))
            .map(|_| HeaderValue::from_static("max-age=31536000; includeSubDomains"));
        Self { csp, hsts }
    }

    pub async fn apply(&self, req: Request, next: Next) -> Response {
        let mut res = next.run(req).await;
        let h = res.headers_mut();
        h.entry(header::CONTENT_SECURITY_POLICY).or_insert_with(|| self.csp.clone());
        h.entry(header::X_CONTENT_TYPE_OPTIONS).or_insert(HeaderValue::from_static("nosniff"));
        h.entry(header::X_FRAME_OPTIONS).or_insert(HeaderValue::from_static("DENY"));
        h.entry(header::REFERRER_POLICY).or_insert(HeaderValue::from_static("strict-origin-when-cross-origin"));
        h.entry(HeaderName::from_static("permissions-policy"))
            .or_insert(HeaderValue::from_static("camera=(self), microphone=(), geolocation=(), payment=(), usb=()"));
        if let Some(hsts) = &self.hsts {
            h.entry(header::STRICT_TRANSPORT_SECURITY).or_insert_with(|| hsts.clone());
        }
        res
    }
}

/// The middleware form, for `Router::layer`.
pub async fn middleware(
    axum::extract::State(headers): axum::extract::State<SecurityHeaders>,
    req: Request,
    next: Next,
) -> Response {
    headers.apply(req, next).await
}

/// The console's policy. Scripts: its own files and the providers' sign-in
/// scripts, nothing inline. Styles allow inline attributes (React sets
/// them). Images and media may come from anywhere: covers and podcast audio
/// are served from a store or a publisher this server does not control, and
/// a home install may be plain http. Fetches (`connect-src`) are limited to
/// this origin, the store the console uploads to, and the providers.
pub fn content_security_policy(cfg: &AppConfig) -> String {
    let mut connect = vec!["'self'".to_string()];
    for endpoint in [cfg.storage.public_endpoint.as_deref(), Some(cfg.storage.endpoint.as_str())] {
        if let Some(origin) = endpoint.and_then(origin_of) {
            if !connect.contains(&origin) {
                connect.push(origin);
            }
        }
    }
    connect.push("https://accounts.google.com".into());
    connect.push("https://appleid.apple.com".into());
    [
        "default-src 'self'".to_string(),
        "script-src 'self' https://accounts.google.com https://appleid.cdn-apple.com".into(),
        "style-src 'self' 'unsafe-inline' https://accounts.google.com".into(),
        "img-src * data: blob:".into(),
        "media-src * data: blob:".into(),
        "font-src 'self' data:".into(),
        format!("connect-src {}", connect.join(" ")),
        "frame-src https://accounts.google.com https://appleid.apple.com".into(),
        "worker-src 'self'".into(),
        "manifest-src 'self'".into(),
        "object-src 'none'".into(),
        "base-uri 'self'".into(),
        "form-action 'self' https://appleid.apple.com".into(),
        "frame-ancestors 'none'".into(),
    ]
    .join("; ")
}

fn origin_of(url: &str) -> Option<String> {
    let url = Url::parse(url.trim()).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(p) => format!("{}://{host}:{p}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_scheme_host_port() {
        assert_eq!(origin_of("http://garage:3900/bucket/x").as_deref(), Some("http://garage:3900"));
        assert_eq!(origin_of("https://s3.example.com").as_deref(), Some("https://s3.example.com"));
        assert_eq!(origin_of(""), None);
    }
}
