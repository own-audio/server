// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign in with Apple in a browser, for apps that have no Apple SDK to ask (Android).
//!
//! Apple sends its answer only to an `https` Return URL registered for a Services ID, never to
//! an app, so the server stands in the middle (RFC 8252's shape, with this server as the
//! authorization server the app talks to):
//!
//! 1. The app opens `GET /api/v2/auth/apple/start` in a browser tab with its PKCE challenge,
//!    its own return address (an allowed custom scheme) and its state. The server records the
//!    sign-in and sends the browser to Apple with a state and a nonce of its own.
//! 2. Apple posts the result to `POST /api/v2/auth/apple/return`. The server checks the id
//!    token (made for the Services ID, carrying the nonce), keeps who signed in and sends the
//!    browser back to the app with a one-time code and the app's state.
//! 3. The app spends the code with its PKCE verifier at `POST /api/v2/auth/token`
//!    (`grant_type=urn:own-audio:params:oauth:grant-type:apple-code`) and gets tokens as from
//!    any other sign-in.
//!
//! Apple's own authorization `code` is never exchanged: that needs a client secret signed with
//! the developer's private key, and the id token alone says who signed in.

use crate::app::config::{AppConfig, AppleAuthConfig};
use crate::auth::error::AuthError;
use crate::auth::oidc::{self, ProviderIdentity};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const AUTHORIZE_URL: &str = "https://appleid.apple.com/auth/authorize";
/// Where Apple posts its answer, under the server's `SERVER__BASE_URL`.
pub const RETURN_PATH: &str = "/api/v2/auth/apple/return";
/// How long a started sign-in waits for Apple, and a code for the app.
pub const TTL_SECS: i64 = 300;

/// The Apple settings when the browser sign-in can run here: Apple on, a Services ID to send
/// people to Apple with, and the server's own public address for Apple to answer at.
pub fn configured(cfg: &AppConfig) -> Option<(&AppleAuthConfig, &str, &str)> {
    let apple = cfg.auth.apple.as_ref().filter(|a| a.enabled)?;
    let client_id = apple.web_client_id.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
    let base = cfg.server.base_url.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
    Some((apple, client_id, base))
}

/// An app's return address is taken only from the allowlist, and only with a scheme of its own:
/// the server never sends a sign-in's code to a web address.
pub fn redirect_allowed(apple: &AppleAuthConfig, redirect_uri: &str) -> bool {
    let Ok(url) = url::Url::parse(redirect_uri) else {
        return false;
    };
    if matches!(url.scheme(), "http" | "https") || url.fragment().is_some() {
        return false;
    }
    apple.app_redirect_uris().iter().any(|allowed| allowed == redirect_uri)
}

/// RFC 7636 §4.2: an S256 challenge is the base64url of a SHA-256, 43 characters.
pub fn challenge_valid(challenge: &str) -> bool {
    challenge.len() == 43 && challenge.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// RFC 7636 §4.1: what a verifier may be.
pub fn verifier_valid(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len()) && verifier.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

/// RFC 7636 §4.6: `BASE64URL(SHA256(verifier)) == challenge`.
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge
}

/// A random secret for a state, a nonce or a code: 32 bytes, base64url.
pub fn secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The server's own Return URL, as registered with Apple for the Services ID.
pub fn return_url(base: &str) -> String {
    format!("{}{RETURN_PATH}", base.trim_end_matches('/'))
}

/// Apple's authorization page for one sign-in. `form_post` is what Apple requires when it is
/// asked for the name and email.
pub fn authorize_url(client_id: &str, return_url: &str, state: &str, nonce: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("a constant URL");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", return_url)
        .append_pair("response_type", "code id_token")
        .append_pair("response_mode", "form_post")
        .append_pair("scope", "name email")
        .append_pair("state", state)
        .append_pair("nonce", nonce);
    // `form_urlencoded` writes spaces as `+`; Apple's documentation writes `%20`, which every
    // decoder reads the same and nobody can misread.
    url.to_string().replace('+', "%20")
}

/// The app's return address with the answer added to its query.
pub fn back_to_app(redirect_uri: &str, pairs: &[(&str, &str)], app_state: Option<&str>) -> String {
    match url::Url::parse(redirect_uri) {
        Ok(mut url) => {
            {
                let mut query = url.query_pairs_mut();
                for (k, v) in pairs {
                    query.append_pair(k, v);
                }
                if let Some(state) = app_state {
                    query.append_pair("state", state);
                }
            }
            url.to_string()
        }
        // Checked against the allowlist when the sign-in started; unreachable in practice.
        Err(_) => redirect_uri.to_string(),
    }
}

/// What Apple posts to the Return URL (form_post).
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct AppleReturn {
    pub state: Option<String>,
    pub id_token: Option<String>,
    /// Apple's authorization code; not used: the id token says who signed in.
    pub code: Option<String>,
    /// JSON, at a person's first sign-in only: `{"name":{"firstName","lastName"},"email"}`.
    pub user: Option<String>,
    /// `user_cancelled_authorize` when the person cancelled.
    pub error: Option<String>,
}

/// The name in Apple's `user` field, which Apple sends once, at the first sign-in.
pub fn full_name(user: Option<&str>) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Name {
        first_name: Option<String>,
        last_name: Option<String>,
    }
    #[derive(Deserialize)]
    struct User {
        name: Option<Name>,
    }
    let name = serde_json::from_str::<User>(user?).ok()?.name?;
    let joined = [name.first_name, name.last_name]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!joined.is_empty()).then_some(joined)
}

/// How a sign-in Apple answered ended.
pub enum Outcome {
    Signed(ProviderIdentity),
    /// The person cancelled, or Apple refused: `access_denied` to the app.
    Denied,
    /// The token did not verify: `server_error` to the app.
    Failed,
}

/// Apple's answer for the sign-in with this nonce, checked.
pub async fn outcome(apple: &AppleAuthConfig, nonce: &str, answer: &AppleReturn) -> Outcome {
    if let Some(error) = answer.error.as_deref().filter(|e| !e.is_empty()) {
        tracing::info!(error, "apple web sign-in: Apple answered with an error");
        return Outcome::Denied;
    }
    let Some(id_token) = answer.id_token.as_deref().filter(|t| !t.is_empty()) else {
        tracing::warn!("apple web sign-in: Apple's answer has no id token");
        return Outcome::Failed;
    };
    match oidc::verify_apple_web_id_token(apple, id_token, nonce).await {
        Ok(mut identity) => {
            identity.display_name = full_name(answer.user.as_deref());
            Outcome::Signed(identity)
        }
        Err(e) => {
            let why = match e {
                AuthError::InvalidCredentials => "the id token did not verify".to_string(),
                other => other.to_string(),
            };
            tracing::warn!(why, "apple web sign-in: refused Apple's answer");
            Outcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::oidc::testing;
    use serde_json::json;

    fn apple(list: Option<&str>) -> AppleAuthConfig {
        AppleAuthConfig {
            enabled: true,
            client_ids: "audio.own.musicplayer,audio.own.web".into(),
            web_client_id: Some("audio.own.web".into()),
            app_redirect_uris: list.map(str::to_string),
        }
    }

    #[test]
    fn only_listed_app_schemes_are_return_addresses() {
        let cfg = apple(None);
        for good in ["audio.own.musicplayer:/apple", "audio.own.bookplayer:/apple", "audio.own.podcastplayer:/apple"] {
            assert!(redirect_allowed(&cfg, good), "{good}");
        }
        for bad in [
            "audio.own.musicplayer:/other",
            "audio.own.musicplayer:/apple?x=1",
            "audio.own.musicplayer:/apple#x",
            "evil.app:/apple",
            "https://evil.example/apple",
            "http://127.0.0.1:8080/apple",
            "",
            "not a uri",
        ] {
            assert!(!redirect_allowed(&cfg, bad), "{bad}");
        }
        // An operator's list replaces the default; an http(s) address there is still refused.
        let own = apple(Some(" com.example.app:/cb , https://example.com/cb"));
        assert!(redirect_allowed(&own, "com.example.app:/cb"));
        assert!(!redirect_allowed(&own, "audio.own.musicplayer:/apple"));
        assert!(!redirect_allowed(&own, "https://example.com/cb"));
        // Blank, as compose passes an unset variable: the default.
        assert!(redirect_allowed(&apple(Some("  ")), "audio.own.musicplayer:/apple"));
    }

    #[test]
    fn the_apple_url_carries_the_services_id_the_return_url_state_and_nonce() {
        let url = authorize_url("audio.own.web", &return_url("https://api.example.com/"), "st", "no");
        assert_eq!(
            url,
            "https://appleid.apple.com/auth/authorize?client_id=audio.own.web\
             &redirect_uri=https%3A%2F%2Fapi.example.com%2Fapi%2Fv2%2Fauth%2Fapple%2Freturn\
             &response_type=code%20id_token&response_mode=form_post&scope=name%20email&state=st&nonce=no"
        );
    }

    #[test]
    fn pkce_is_s256() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(challenge_valid(challenge) && verifier_valid(verifier));
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches(&verifier.replace('d', "e"), challenge));
        assert!(!challenge_valid("short") && !challenge_valid(&format!("{}+", "a".repeat(42))));
        assert!(!verifier_valid("short") && !verifier_valid(&format!("{}!", "a".repeat(43))));
    }

    #[test]
    fn the_app_gets_its_answer_and_its_own_state() {
        assert_eq!(
            back_to_app("audio.own.musicplayer:/apple", &[("code", "c-1")], Some("app st")),
            "audio.own.musicplayer:/apple?code=c-1&state=app+st"
        );
        assert_eq!(
            back_to_app("audio.own.musicplayer:/apple", &[("error", "access_denied")], None),
            "audio.own.musicplayer:/apple?error=access_denied"
        );
    }

    #[test]
    fn apples_first_time_name_is_read_when_there_is_one() {
        let user = r#"{"name":{"firstName":"Anna","lastName":"Nováková"},"email":"a@example.com"}"#;
        assert_eq!(full_name(Some(user)).as_deref(), Some("Anna Nováková"));
        assert_eq!(full_name(Some(r#"{"name":{"firstName":" Anna "}}"#)).as_deref(), Some("Anna"));
        assert_eq!(full_name(Some(r#"{"email":"a@example.com"}"#)), None);
        assert_eq!(full_name(Some("not json")), None);
        assert_eq!(full_name(None), None);
    }

    fn token(nonce: &str) -> String {
        testing::sign(json!({
            "sub": "001234.apple-user",
            "email": "anna@example.com",
            "email_verified": true,
            "iss": "https://appleid.apple.com",
            "aud": "audio.own.web",
            "nonce": nonce,
            "exp": (chrono::Utc::now() + chrono::Duration::minutes(10)).timestamp(),
        }))
    }

    #[tokio::test]
    async fn apples_answer_signs_in_cancels_or_fails() {
        testing::seed_apple_jwks().await;
        let cfg = apple(None);
        let answer = AppleReturn {
            state: Some("st".into()),
            id_token: Some(token("n-1")),
            user: Some(r#"{"name":{"firstName":"Anna","lastName":"N"}}"#.into()),
            ..Default::default()
        };
        match outcome(&cfg, "n-1", &answer).await {
            Outcome::Signed(identity) => {
                assert_eq!(identity.subject, "001234.apple-user");
                assert_eq!(identity.display_name.as_deref(), Some("Anna N"));
            }
            _ => panic!("expected a sign-in"),
        }
        assert!(matches!(outcome(&cfg, "n-2", &answer).await, Outcome::Failed), "another sign-in's nonce");
        let cancelled = AppleReturn { state: Some("st".into()), error: Some("user_cancelled_authorize".into()), ..Default::default() };
        assert!(matches!(outcome(&cfg, "n-1", &cancelled).await, Outcome::Denied));
        let empty = AppleReturn { state: Some("st".into()), ..Default::default() };
        assert!(matches!(outcome(&cfg, "n-1", &empty).await, Outcome::Failed));
        let garbage = AppleReturn { id_token: Some("not.a.jwt".into()), ..Default::default() };
        assert!(matches!(outcome(&cfg, "n-1", &garbage).await, Outcome::Failed));
    }
}
