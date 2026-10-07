// SPDX-License-Identifier: AGPL-3.0-or-later
//! Links to the server's own media route, `/api/v1/media`, signed like a
//! presigned S3 URL: the key, the method and an expiry, under an HMAC only
//! this server can make. They stand in for presigned URLs wherever the store
//! cannot hand out links of its own (local storage) or should not (a network
//! that blocks the store's host), so clients see the same opaque, expiring
//! URL either way and never change.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The route the links point at, under the API prefix.
pub const MEDIA_PATH: &str = "/api/v1/media";

#[derive(Clone)]
pub struct MediaLinks {
    key: Vec<u8>,
    /// Absolute origin (`https://demo.own.audio`), or empty for a relative
    /// link, which only the web console on the same origin can follow.
    base: String,
}

impl MediaLinks {
    /// `secret` is the session secret; the signing key is derived from it so
    /// a media link can never be mistaken for a session token or the reverse.
    pub fn new(secret: &str, base_url: Option<&str>) -> Self {
        let mut h = Sha256::new();
        h.update(b"own.audio media links v1\0");
        h.update(secret.as_bytes());
        Self { key: h.finalize().to_vec(), base: base_url.unwrap_or("").trim_end_matches('/').to_string() }
    }

    fn signature(&self, method: &str, key: &str, expires: i64) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(method.as_bytes());
        mac.update(b"\n");
        mac.update(key.as_bytes());
        mac.update(b"\n");
        mac.update(expires.to_string().as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// A link for `method` (`GET` or `PUT`) on `key`, valid for `ttl_secs`.
    pub fn link(&self, method: &str, key: &str, ttl_secs: u64) -> String {
        let expires = chrono::Utc::now().timestamp() + ttl_secs as i64;
        let sig = self.signature(method, key, expires);
        format!(
            "{}{MEDIA_PATH}?k={}&e={expires}&s={sig}",
            self.base,
            urlencoding::encode(key)
        )
    }

    /// Whether a link's parameters are genuine and unexpired. `HEAD` is
    /// accepted on a `GET` link, as S3 does.
    pub fn verify(&self, method: &str, key: &str, expires: i64, sig: &str) -> bool {
        if expires < chrono::Utc::now().timestamp() {
            return false;
        }
        let method = if method == "HEAD" { "GET" } else { method };
        let Ok(given) = URL_SAFE_NO_PAD.decode(sig) else {
            return false;
        };
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(method.as_bytes());
        mac.update(b"\n");
        mac.update(key.as_bytes());
        mac.update(b"\n");
        mac.update(expires.to_string().as_bytes());
        mac.verify_slice(&given).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(link: &str) -> (String, i64, String) {
        let query = link.split_once('?').unwrap().1;
        let mut k = String::new();
        let mut e = 0;
        let mut s = String::new();
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=').unwrap();
            match name {
                "k" => k = urlencoding::decode(value).unwrap().into_owned(),
                "e" => e = value.parse().unwrap(),
                "s" => s = value.to_string(),
                _ => {}
            }
        }
        (k, e, s)
    }

    #[test]
    fn a_link_verifies_for_its_method_and_key() {
        let links = MediaLinks::new("secret", Some("https://demo.own.audio/"));
        let link = links.link("GET", "f/1/music/a b.mp3", 60);
        assert!(link.starts_with("https://demo.own.audio/api/v1/media?k=f%2F1%2Fmusic%2Fa%20b.mp3&e="));
        let (k, e, s) = parts(&link);
        assert!(links.verify("GET", &k, e, &s));
        assert!(links.verify("HEAD", &k, e, &s));
        assert!(!links.verify("PUT", &k, e, &s), "a read link must not allow writing");
        assert!(!links.verify("GET", "f/1/music/other.mp3", e, &s));
        assert!(!links.verify("GET", &k, e + 1, &s), "the expiry is signed");
    }

    #[test]
    fn another_secret_or_an_old_link_is_refused() {
        let links = MediaLinks::new("secret", None);
        let (k, e, s) = parts(&links.link("GET", "x", 60));
        assert!(!MediaLinks::new("other", None).verify("GET", &k, e, &s));
        let past = chrono::Utc::now().timestamp() - 1;
        let sig = links.signature("GET", "x", past);
        assert!(!links.verify("GET", "x", past, &sig));
    }

    #[test]
    fn without_a_base_url_the_link_is_relative() {
        let link = MediaLinks::new("s", None).link("GET", "x", 60);
        assert!(link.starts_with("/api/v1/media?k=x&e="));
    }
}
