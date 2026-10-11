// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secrets the server has to keep readable, kept unreadable in the database
//! (security hardening plan §5.2). The Subsonic API key is the case: the
//! protocol's token is `md5(key + salt)`, so the key cannot be hashed the
//! way a password is — it is encrypted instead, with a key derived from
//! `AUTH__SESSION_SECRET`, and a database dump alone gives nothing.
//!
//! Stored form: `enc1:` + base64url(nonce || ciphertext). A value without
//! the prefix is one written before this existed and is read as plaintext,
//! then rewritten encrypted on first use and by the start-up sweep.
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use anyhow::{bail, Context};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hkdf::Hkdf;
use sha2::Sha256;

const PREFIX: &str = "enc1:";

#[derive(Clone)]
pub struct Cipher {
    key: Key<Aes256Gcm>,
}

impl Cipher {
    /// One cipher per secret and purpose; `info` keeps keys for different
    /// uses apart even though they share the secret.
    pub fn derive(secret: &str, info: &str) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(b"own.audio at-rest"), secret.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(info.as_bytes(), &mut key).expect("32 bytes is a valid HKDF-SHA256 length");
        Self { key: key.into() }
    }

    pub fn seal(&self, plain: &str) -> String {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let cipher = Aes256Gcm::new(&self.key);
        let sealed = cipher.encrypt(&nonce, plain.as_bytes()).expect("AES-GCM encrypt");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(out))
    }

    /// The plaintext, and whether the stored value was already encrypted.
    pub fn open(&self, stored: &str) -> anyhow::Result<(String, bool)> {
        let Some(encoded) = stored.strip_prefix(PREFIX) else {
            return Ok((stored.to_string(), false));
        };
        let bytes = URL_SAFE_NO_PAD.decode(encoded).context("at-rest value is not base64")?;
        if bytes.len() < 12 {
            bail!("at-rest value is too short");
        }
        let (nonce, sealed) = bytes.split_at(12);
        let cipher = Aes256Gcm::new(&self.key);
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), sealed)
            .map_err(|_| anyhow::anyhow!("at-rest value does not open with this server's secret"))?;
        Ok((String::from_utf8(plain).context("at-rest value is not UTF-8")?, true))
    }

    pub fn is_sealed(stored: &str) -> bool {
        stored.starts_with(PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_plaintext_pass_through() {
        let c = Cipher::derive("a secret", "subsonic");
        let sealed = c.seal("the-api-key");
        assert!(Cipher::is_sealed(&sealed));
        assert_ne!(sealed, c.seal("the-api-key"), "a fresh nonce every time");
        assert_eq!(c.open(&sealed).unwrap(), ("the-api-key".to_string(), true));
        assert_eq!(c.open("legacy-plain").unwrap(), ("legacy-plain".to_string(), false));
    }

    #[test]
    fn another_secret_or_purpose_does_not_open_it() {
        let sealed = Cipher::derive("a secret", "subsonic").seal("k");
        assert!(Cipher::derive("another", "subsonic").open(&sealed).is_err());
        assert!(Cipher::derive("a secret", "other").open(&sealed).is_err());
    }
}
