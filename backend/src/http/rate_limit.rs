// SPDX-License-Identifier: AGPL-3.0-or-later
//! Request-rate limits (`RateLimitConfig`): per address on the guessable
//! routes (sign-in, device codes, join codes, setup), per caller on the whole
//! API with stricter buckets for search and for routes that make the server
//! call out (security hardening plan §6).
//!
//! A `Limiter` wraps one keyed GCRA limiter from the `governor` crate and is
//! applied to a `MethodRouter` with [`Limiter::apply`] or to a whole router
//! with [`Limiter::wrap`]. The key is the bearer token when there is one —
//! each signed-in device has its own budget, whatever address it comes from
//! — and the client address otherwise. A refused request answers
//! `429 rate_limited` with `Retry-After`, the one status every client
//! already treats as "wait".

use crate::app::AppState;
use crate::app::config::RateLimitConfig;
use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use governor::clock::{Clock, DefaultClock};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use serde_json::json;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone)]
pub struct Limiter {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    limiter: DefaultKeyedRateLimiter<String>,
    trust_proxy_headers: bool,
    checks: AtomicU64,
}

impl Limiter {
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn per_minute(n: u32, trust_proxy_headers: bool) -> Self {
        let Some(n) = NonZeroU32::new(n) else {
            return Self::disabled();
        };
        // Replenish one permit every 60/n seconds, allow n in a burst: "n per
        // minute" the way a person reads it, without a hard window edge.
        let quota = Quota::with_period(Duration::from_secs_f64(60.0 / n.get() as f64))
            .expect("non-zero period")
            .allow_burst(n);
        Self {
            inner: Some(Arc::new(Inner {
                limiter: RateLimiter::keyed(quota),
                trust_proxy_headers,
                checks: AtomicU64::new(0),
            })),
        }
    }

    /// Wrap the handlers of one route. A disabled limiter returns them unchanged.
    pub fn apply(&self, route: MethodRouter<AppState>) -> MethodRouter<AppState> {
        match &self.inner {
            Some(_) => route.route_layer(middleware::from_fn_with_state(self.clone(), check)),
            None => route,
        }
    }

    /// Wrap every route of a router (matched routes only: a 404 costs nothing).
    pub fn wrap(&self, router: axum::Router<AppState>) -> axum::Router<AppState> {
        match &self.inner {
            Some(_) => router.route_layer(middleware::from_fn_with_state(self.clone(), check)),
            None => router,
        }
    }
}

/// The limiters the core's routers take, one per protected route group.
#[derive(Clone)]
pub struct Limiters {
    pub login: Limiter,
    pub refresh: Limiter,
    pub device: Limiter,
    pub join: Limiter,
    pub setup: Limiter,
    /// The whole `/api/v1` and `/rest`, per caller.
    pub api: Limiter,
    /// Library and catalogue searches.
    pub search: Limiter,
    /// Routes that make the server fetch from elsewhere: feeds, identification, metadata lookups.
    pub outbound: Limiter,
}

impl Limiters {
    pub fn from_config(cfg: &RateLimitConfig) -> Self {
        if !cfg.enabled {
            return Self::disabled();
        }
        let t = cfg.trust_proxy_headers;
        Self {
            login: Limiter::per_minute(cfg.login_per_minute, t),
            refresh: Limiter::per_minute(cfg.refresh_per_minute, t),
            device: Limiter::per_minute(cfg.device_per_minute, t),
            join: Limiter::per_minute(cfg.join_per_minute, t),
            setup: Limiter::per_minute(cfg.setup_per_minute, t),
            api: Limiter::per_minute(cfg.api_per_minute, t),
            search: Limiter::per_minute(cfg.search_per_minute, t),
            outbound: Limiter::per_minute(cfg.outbound_per_minute, t),
        }
    }

    pub fn disabled() -> Self {
        Self {
            login: Limiter::disabled(),
            refresh: Limiter::disabled(),
            device: Limiter::disabled(),
            join: Limiter::disabled(),
            setup: Limiter::disabled(),
            api: Limiter::disabled(),
            search: Limiter::disabled(),
            outbound: Limiter::disabled(),
        }
    }
}

async fn check(State(limiter): State<Limiter>, req: Request, next: Next) -> Response {
    let Some(inner) = limiter.inner.as_ref() else {
        return next.run(req).await;
    };
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let key = request_key(peer, req.headers(), inner.trust_proxy_headers);
    // The keyed store only grows; drop idle keys now and then.
    if inner.checks.fetch_add(1, Ordering::Relaxed) % 1024 == 1023 {
        inner.limiter.retain_recent();
    }
    match inner.limiter.check_key(&key) {
        Ok(()) => next.run(req).await,
        Err(not_until) => {
            let wait = not_until.wait_time_from(DefaultClock::default().now());
            let secs = wait.as_secs().max(1);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("Retry-After", secs.to_string())],
                Json(json!({ "error": "rate_limited", "retry_after_secs": secs })),
            )
                .into_response()
        }
    }
}

/// A signed-in device by its bearer token (hashed; the token itself is never
/// a map key), anyone else by address.
pub fn request_key(peer: Option<IpAddr>, headers: &HeaderMap, trust_proxy_headers: bool) -> String {
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty())
    {
        return format!("t:{}", &crate::db::refresh_tokens::hash_token(token)[..32]);
    }
    format!("ip:{}", client_ip(peer, headers, trust_proxy_headers))
}

fn is_private_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local() || v6.is_unspecified(),
    }
}

/// Which address a request counts against. Proxy headers are believed only
/// when the peer is a proxy we can plausibly be behind (private/loopback) or
/// the operator said so; a public peer is always keyed by itself.
pub fn client_ip(peer: Option<IpAddr>, headers: &HeaderMap, trust_proxy_headers: bool) -> IpAddr {
    let fallback = peer.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let behind_proxy = trust_proxy_headers || peer.is_none_or(is_private_or_loopback);
    if !behind_proxy {
        return fallback;
    }
    let header_ip = |name: &str, first_only: bool| -> Option<IpAddr> {
        let v = headers.get(name)?.to_str().ok()?;
        let v = if first_only { v.split(',').next().unwrap_or(v) } else { v };
        v.trim().parse().ok()
    };
    header_ip("cf-connecting-ip", false)
        .or_else(|| header_ip("x-forwarded-for", true))
        .or_else(|| header_ip("x-real-ip", false))
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_bearer_token_is_the_key_when_present() {
        let mut h = HeaderMap::new();
        assert_eq!(request_key(Some(ip("203.0.113.9")), &h, false), "ip:203.0.113.9");
        h.insert("authorization", HeaderValue::from_static("Bearer abc"));
        let k = request_key(Some(ip("203.0.113.9")), &h, false);
        assert!(k.starts_with("t:") && !k.contains("abc"), "{k}");
        assert_eq!(k, request_key(Some(ip("198.51.100.1")), &h, false), "the same device from another address");
    }

    #[test]
    fn public_peer_is_keyed_by_itself_even_with_forwarded_headers() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        assert_eq!(client_ip(Some(ip("203.0.113.9")), &h, false), ip("203.0.113.9"));
    }

    #[test]
    fn private_peer_means_a_proxy_so_the_forwarded_client_counts() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4, 10.0.0.2"));
        assert_eq!(client_ip(Some(ip("172.18.0.5")), &h, false), ip("1.2.3.4"));
    }

    #[test]
    fn cloudflare_header_wins_when_trusted() {
        let mut h = HeaderMap::new();
        h.insert("cf-connecting-ip", HeaderValue::from_static("2001:db8::7"));
        h.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        assert_eq!(client_ip(Some(ip("203.0.113.9")), &h, true), ip("2001:db8::7"));
    }

    #[test]
    fn garbage_headers_fall_back_to_the_peer() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert_eq!(client_ip(Some(ip("127.0.0.1")), &h, false), ip("127.0.0.1"));
    }

    #[test]
    fn limiter_refuses_after_the_burst() {
        let l = Limiter::per_minute(3, false);
        let inner = l.inner.as_ref().unwrap();
        let k = "ip:198.51.100.1".to_string();
        assert!(inner.limiter.check_key(&k).is_ok());
        assert!(inner.limiter.check_key(&k).is_ok());
        assert!(inner.limiter.check_key(&k).is_ok());
        assert!(inner.limiter.check_key(&k).is_err());
        assert!(inner.limiter.check_key(&"ip:198.51.100.2".to_string()).is_ok());
    }
}
