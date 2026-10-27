// SPDX-License-Identifier: AGPL-3.0-or-later
//! Links to the media proxy: a second way to the objects in S3, for networks
//! that block the store's own host (company firewalls that block
//! `*.r2.cloudflarestorage.com`, say). The proxy lives on the operator's own
//! domain, holds the same key, checks the signature and reads the bucket.
//!
//! The link: `{base}/{key}?e={expires}&s={signature}`, the key path-encoded
//! segment by segment, the signature HMAC-SHA256 over `GET\n{key}\n{expires}`
//! in URL-safe base64 without padding. Anything that checks it must build the
//! same message — the hosted edition's media Worker does.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

#[derive(Clone)]
pub struct MediaProxy {
    base: String,
    secret: String,
}

impl MediaProxy {
    /// `None` unless both the address and the key are set.
    pub fn new(base: Option<&str>, secret: Option<&str>) -> Option<Self> {
        let base = base?.trim().trim_end_matches('/');
        let secret = secret?.trim();
        (!base.is_empty() && !secret.is_empty()).then(|| Self { base: base.to_string(), secret: secret.to_string() })
    }

    pub fn link(&self, key: &str, expires_in_secs: u64) -> String {
        let expires = chrono::Utc::now().timestamp() + expires_in_secs as i64;
        self.link_at(key, expires)
    }

    fn link_at(&self, key: &str, expires: i64) -> String {
        let path: Vec<String> = key.split('/').map(|s| urlencoding::encode(s).into_owned()).collect();
        format!("{}/{}?e={expires}&s={}", self.base, path.join("/"), signature(&self.secret, key, expires))
    }
}

pub fn signature(secret: &str, key: &str, expires: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key length");
    mac.update(format!("GET\n{key}\n{expires}").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_with_both_an_address_and_a_key() {
        assert!(MediaProxy::new(None, Some("k")).is_none());
        assert!(MediaProxy::new(Some("https://media.example"), None).is_none());
        assert!(MediaProxy::new(Some(" "), Some("k")).is_none());
        assert!(MediaProxy::new(Some("https://media.example/"), Some("k")).is_some());
    }

    #[test]
    fn a_link_names_the_key_and_is_signed_for_it() {
        let proxy = MediaProxy::new(Some("https://media.example/"), Some("secret")).unwrap();
        let link = proxy.link_at("families/f1/tracks/a b+c.flac", 1_800_000_000);
        assert_eq!(
            link,
            format!(
                "https://media.example/families/f1/tracks/a%20b%2Bc.flac?e=1800000000&s={}",
                signature("secret", "families/f1/tracks/a b+c.flac", 1_800_000_000)
            )
        );
        assert_ne!(signature("secret", "a", 1), signature("other", "a", 1), "another key");
        assert_ne!(signature("secret", "a", 1), signature("secret", "b", 1), "another object");
        assert_ne!(signature("secret", "a", 1), signature("secret", "a", 2), "another expiry");
    }

    /// The message the Worker checks; a change here breaks every link it serves.
    #[test]
    fn the_signature_is_stable() {
        assert_eq!(signature("k", "x/y.mp3", 1), "lhVJ1gy4Wrwnvge8RZuTxoFGnF3FwpfM5C5vB3UmZp8");
    }
}
