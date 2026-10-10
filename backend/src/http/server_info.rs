// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/v1/server` — what this server is and what it offers, so a
//! client asks instead of assuming. The contract is
//! `own-audio-foss/docs/API_COMPATIBILITY.md` §3: every optional capability
//! has a key under `features`, `true` only when fully configured; unknown
//! keys are ignored by clients and missing keys read as `false`.

use crate::app::AppState;
use axum::Json;
use axum::extract::State;
use serde_json::{Map, Value, json};

/// The path major, `/api/v1`.
pub const API_VERSION: u32 = 1;

/// Bumped by every additive change to the v1 contract (new endpoint, field,
/// enum value or `features` key) — the one number a client may compare.
/// 1: this endpoint, `GET /family/storage`, the `feature_unavailable` 501.
/// 2: `demo` in this endpoint's response.
/// 3: `source` and `read_only` on tracks and books; `GET /library/folders`,
///    `POST /library/folders/scan`; stream URLs may be server media links.
/// 4: `features.one_family`.
/// 5: `features.podcast_search`.
/// 6: `id` and `addresses` in this endpoint's response; `maxBitRate`,
///    `format` and `timeOffset` on Subsonic `stream`.
pub const API_REVISION: u32 = 7;

/// What this server is and offers: edition, version, contract revision and
/// `features`. Clients read it once after `GET /setup/status`; a 404 means a
/// server older than revision 1 (docs/API_COMPATIBILITY.md §3).
#[utoipa::path(get, path = "/server", tag = "server",
    responses((status = 200, description = "`id`, `name`, `edition`, `version`, `api` {version, revision}, `addresses` [{url, scope}], `features`, `deprecations`, optional `demo`", body = Object)))]
pub async fn server_info(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config();
    let providers = crate::auth::providers_response(&cfg.auth);

    let mut features = Map::new();
    features.insert("registration_open".into(), json!(cfg.auth.registration_open));
    features.insert(
        "auth".into(),
        json!({
            "local": providers.local,
            "google": providers.google.enabled,
            "apple": providers.apple.enabled,
            "microsoft": providers.microsoft.enabled,
            // Revision 7: "forgot password" links can be mailed, and new
            // accounts get a confirmation link (`me.email_verified`).
            "password_reset": crate::auth::password_reset_offered(cfg),
            "email_verification": crate::auth::verification::offered(cfg),
        }),
    );
    features.insert("uploads".into(), json!({ "presigned": true, "multipart_max_bytes": Value::Null }));
    // Identify: the metadata service, or else the public MusicBrainz API
    // (unless MUSICBRAINZ__ENABLED=false). Podcast discovery (search,
    // categories, similar shows) needs the metadata service's catalogue.
    features.insert("music_identify".into(), json!(cfg.metadata.is_some() || cfg.musicbrainz.enabled));
    features.insert("podcast_discovery".into(), json!(cfg.metadata.is_some()));
    // Revision 5: search alone, which Apple's public search can answer.
    features.insert("podcast_search".into(), json!(cfg.metadata.is_some() || cfg.itunes.enabled));
    features.insert("file_sync".into(), json!(true));
    features.insert(
        "library_folders".into(),
        json!(crate::library_folders::configured(cfg).is_ok_and(|f| !f.is_empty())),
    );
    features.insert("subsonic".into(), json!(true));
    let mail = cfg.mail.as_ref().is_some_and(|m| m.smtp_host().is_some());
    features.insert("mail".into(), json!(mail));
    // Revision 4: every account joins the one family; clients hide "leave".
    features.insert("one_family".into(), json!(state.hooks().one_family()));
    // The edition's own keys; absent in the open-source edition, so set the
    // documented defaults first and let the hooks overwrite.
    for key in ["billing", "payments", "narration", "translation"] {
        features.insert(key.into(), json!(false));
    }
    for (k, v) in state.hooks().features() {
        features.insert(k, v);
    }

    // Only a public demo sets this; everywhere else the key is absent.
    let demo = cfg.server.demo.as_ref().map(|d| json!({ "email": d.email, "password": d.password }));

    // A database without the row is a broken install; the key is then
    // absent rather than the whole endpoint failing, since clients ask this
    // before anything else.
    let id = crate::db::instance::id(state.db()).await.ok();

    let mut body = json!({
        "name": "own.audio",
        "edition": state.hooks().edition(),
        "version": env!("CARGO_PKG_VERSION"),
        "api": { "version": API_VERSION, "revision": API_REVISION },
        "addresses": addresses(cfg.server.base_url.as_deref(), cfg.server.addresses.as_deref()),
        "features": Value::Object(features),
        "deprecations": [],
    });
    if let Some(id) = id {
        body["id"] = json!(id);
    }
    if let Some(demo) = demo {
        body["demo"] = demo;
    }
    Json(body)
}

/// `base_url` first, then `SERVER__ADDRESSES`, each once, with where it is
/// reachable from. A client races them and keeps the fastest that answers with
/// the same `id`; `scope` only tells it what to expect (a `lan` address is
/// worth trying at home, a `vpn` one when a VPN is up).
fn addresses(base_url: Option<&str>, extra: Option<&str>) -> Value {
    let mut seen = Vec::<String>::new();
    let candidates = base_url.into_iter().chain(extra.unwrap_or_default().split(','));
    for raw in candidates {
        let url = raw.trim().trim_end_matches('/');
        if url.is_empty() || seen.iter().any(|s| s == url) {
            continue;
        }
        seen.push(url.to_string());
    }
    Value::Array(
        seen.into_iter()
            .map(|url| {
                let scope = scope(&url);
                json!({ "url": url, "scope": scope })
            })
            .collect(),
    )
}

/// `lan` for private, link-local and `.local` hosts; `vpn` for the carrier-grade
/// NAT range Tailscale uses and `*.ts.net`; `public` otherwise.
fn scope(url: &str) -> &'static str {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?'])
        .next()
        .unwrap_or_default();
    // An IPv6 literal is in brackets; otherwise the port follows the last colon.
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    let host = host.to_ascii_lowercase();

    if host == "localhost" || host.ends_with(".local") || host.ends_with(".lan") || host.ends_with(".home.arpa") {
        return "lan";
    }
    if host.ends_with(".ts.net") {
        return "vpn";
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let [a, b, ..] = ip.octets();
            if a == 100 && (64..=127).contains(&b) {
                "vpn"
            } else if ip.is_private() || ip.is_link_local() || ip.is_loopback() {
                "lan"
            } else {
                "public"
            }
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            // fc00::/7 unique local, fe80::/10 link-local.
            if ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
                "lan"
            } else {
                "public"
            }
        }
        Err(_) => "public",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes() {
        assert_eq!(scope("http://192.168.1.20:8080"), "lan");
        assert_eq!(scope("http://10.0.0.5"), "lan");
        assert_eq!(scope("http://nas.local:8080"), "lan");
        assert_eq!(scope("http://[fd12::1]:8080"), "lan");
        assert_eq!(scope("http://100.101.102.103:8080"), "vpn");
        assert_eq!(scope("https://nas.tail1234.ts.net"), "vpn");
        assert_eq!(scope("https://music.example.com"), "public");
        assert_eq!(scope("https://8.8.8.8"), "public");
    }

    #[test]
    fn addresses_dedupe_and_keep_order() {
        let list = addresses(
            Some("https://music.example.com/"),
            Some(" http://192.168.1.20:8080 , https://music.example.com,, http://100.70.0.1:8080"),
        );
        let urls: Vec<&str> = list.as_array().unwrap().iter().map(|a| a["url"].as_str().unwrap()).collect();
        assert_eq!(urls, ["https://music.example.com", "http://192.168.1.20:8080", "http://100.70.0.1:8080"]);
        assert_eq!(list[1]["scope"], "lan");
        assert_eq!(list[2]["scope"], "vpn");
        assert_eq!(addresses(None, None), json!([]));
    }
}
