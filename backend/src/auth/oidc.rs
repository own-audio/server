// SPDX-License-Identifier: AGPL-3.0-or-later
//! Google + Apple ID-token verification for native sign-in.
//!
//! Both providers publish their signing keys as a JWKS; verification is
//! signature + `iss`/`aud`/`exp` checks against a short-lived in-process
//! cache of that key set, no network round-trip on the hot path. There is no
//! browser-redirect flow here — clients obtain an ID token themselves (Apple:
//! native `ASAuthorizationController`; Google: the loopback+PKCE code
//! exchange in [`exchange_google_code`]) and this module only verifies it.

use crate::app::config::{AppleAuthConfig, GoogleAuthConfig, MicrosoftAuthConfig};
use crate::auth::error::AuthError;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const MICROSOFT_JWKS_URL: &str = "https://login.microsoftonline.com/common/discovery/v2.0/keys";
const MICROSOFT_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const JWKS_TTL: Duration = Duration::from_secs(3600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

static GOOGLE_JWKS_CACHE: RwLock<Option<CachedJwks>> = RwLock::const_new(None);
static APPLE_JWKS_CACHE: RwLock<Option<CachedJwks>> = RwLock::const_new(None);
static MICROSOFT_JWKS_CACHE: RwLock<Option<CachedJwks>> = RwLock::const_new(None);

struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
}

/// A verified identity from a provider's ID token, normalized across
/// providers. `display_name` is Google's `name` claim; Apple's identity
/// token carries no name claim, so it is always `None` there — the caller
/// (`auth::mod::apple_sign_in`) merges in the client-supplied full name
/// instead, which Apple only sends on a user's first-ever authorization.
pub struct ProviderIdentity {
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub display_name: Option<String>,
}

pub async fn verify_google_id_token(
    cfg: &GoogleAuthConfig,
    id_token: &str,
) -> Result<ProviderIdentity, AuthError> {
    let claims = verify_rs256(
        GOOGLE_JWKS_URL,
        &GOOGLE_JWKS_CACHE,
        &["https://accounts.google.com", "accounts.google.com"],
        &allowed_audiences(&cfg.client_ids),
        id_token,
    )
    .await?;

    Ok(ProviderIdentity {
        subject: claims.sub,
        email: claims.email.unwrap_or_default(),
        email_verified: claims.email_verified,
        display_name: claims.name,
    })
}

pub async fn verify_apple_id_token(
    cfg: &AppleAuthConfig,
    identity_token: &str,
) -> Result<ProviderIdentity, AuthError> {
    let claims = verify_rs256(
        APPLE_JWKS_URL,
        &APPLE_JWKS_CACHE,
        &["https://appleid.apple.com"],
        &allowed_audiences(&cfg.client_ids),
        identity_token,
    )
    .await?;

    Ok(ProviderIdentity {
        subject: claims.sub,
        email: claims.email.unwrap_or_default(),
        // Apple only issues verified-email accounts through Sign in with
        // Apple; still read the claim rather than assuming, since Apple's
        // private-relay addresses set it explicitly too.
        email_verified: claims.email_verified,
        display_name: None,
    })
}

/// Exchange a loopback authorization code for an ID token. Only the backend
/// holds `desktop_client_secret` — the Mac app never sees it. `redirect_uri`
/// must already have been checked as loopback-only by the caller
/// (`auth::mod::is_loopback_redirect`); this function does not re-check it.
pub async fn exchange_google_code(
    cfg: &GoogleAuthConfig,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<String, AuthError> {
    let client_id = cfg.desktop_client_id.as_deref().ok_or_else(|| {
        AuthError::BadRequest("Google desktop client is not configured".into())
    })?;
    let client_secret = cfg.desktop_client_secret.as_deref().ok_or_else(|| {
        AuthError::BadRequest("Google desktop client is not configured".into())
    })?;

    let client = http_client()?;
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("google token exchange failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("google token exchange rejected ({status}): {body}");
        return Err(AuthError::BadRequest("Google sign-in failed".into()));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        id_token: String,
    }

    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("google token response invalid: {e}")))?;
    Ok(token.id_token)
}

fn allowed_audiences(client_ids: &str) -> Vec<String> {
    client_ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn http_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| AuthError::Internal(anyhow::anyhow!(e)))
}

async fn fetch_jwks(url: &str) -> Result<JwkSet, AuthError> {
    let resp = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("jwks fetch failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AuthError::Internal(anyhow::anyhow!(
            "jwks fetch returned {}",
            resp.status()
        )));
    }

    resp.json()
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("jwks response invalid: {e}")))
}

async fn cached_jwks(url: &str, cache: &'static RwLock<Option<CachedJwks>>) -> Result<JwkSet, AuthError> {
    {
        let guard = cache.read().await;
        if let Some(cached) = guard.as_ref() {
            if cached.fetched_at.elapsed() < JWKS_TTL {
                return Ok(cached.set.clone());
            }
        }
    }

    let fresh = fetch_jwks(url).await?;
    let mut guard = cache.write().await;
    guard.replace(CachedJwks { set: fresh.clone(), fetched_at: Instant::now() });
    Ok(fresh)
}

/// Look up `kid` in the cached key set; on a miss (key rotation), refetch
/// once bypassing the cache before giving up.
async fn find_key(
    url: &str,
    cache: &'static RwLock<Option<CachedJwks>>,
    kid: &str,
) -> Result<jsonwebtoken::jwk::Jwk, AuthError> {
    let set = cached_jwks(url, cache).await?;
    if let Some(jwk) = set.find(kid) {
        return Ok(jwk.clone());
    }

    let fresh = fetch_jwks(url).await?;
    let found = fresh.find(kid).cloned();
    let mut guard = cache.write().await;
    guard.replace(CachedJwks { set: fresh, fetched_at: Instant::now() });
    found.ok_or(AuthError::InvalidCredentials)
}

#[derive(Debug, Deserialize)]
struct IdTokenClaims {
    sub: String,
    /// Read back for Microsoft, whose issuer depends on `tid` and so cannot be
    /// checked by `jsonwebtoken`'s own issuer validation.
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// Google always sends a JSON bool; Apple has sent both a JSON bool and
    /// a `"true"`/`"false"` string across SDK versions — accept either.
    #[serde(default, deserialize_with = "bool_or_string")]
    email_verified: bool,
    #[serde(default)]
    name: Option<String>,
    /// Microsoft only: the tenant the token was issued for. The `common`
    /// authority's `iss` is built from it, so it is what makes the issuer
    /// check possible at all — see [`verify_microsoft_id_token`].
    #[serde(default)]
    tid: Option<String>,
    /// Microsoft only: usually the email address, and sometimes the only
    /// place one appears.
    #[serde(default)]
    preferred_username: Option<String>,
}

fn bool_or_string<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrString {
        Bool(bool),
        String(String),
    }
    Ok(match BoolOrString::deserialize(deserializer)? {
        BoolOrString::Bool(b) => b,
        BoolOrString::String(s) => s == "true",
    })
}

async fn verify_rs256(
    jwks_url: &str,
    cache: &'static RwLock<Option<CachedJwks>>,
    allowed_issuers: &[&str],
    allowed_audiences: &[String],
    token: &str,
) -> Result<IdTokenClaims, AuthError> {
    if allowed_audiences.is_empty() {
        // Configured with an empty client_ids list — treat as unconfigured
        // rather than accepting a token from anywhere.
        return Err(AuthError::ProviderNotConfigured);
    }

    let header = decode_header(token).map_err(|_| AuthError::InvalidCredentials)?;
    let kid = header.kid.ok_or(AuthError::InvalidCredentials)?;
    let jwk = find_key(jwks_url, cache, &kid).await?;
    verify_claims(&jwk, allowed_issuers, allowed_audiences, token)
}

/// The pure signature+claims check, split out from [`verify_rs256`] so it
/// can run against a hand-built [`Jwk`] in tests without a network call.
fn verify_claims(
    jwk: &jsonwebtoken::jwk::Jwk,
    allowed_issuers: &[&str],
    allowed_audiences: &[String],
    token: &str,
) -> Result<IdTokenClaims, AuthError> {
    let key = DecodingKey::from_jwk(jwk).map_err(|_| AuthError::InvalidCredentials)?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(allowed_audiences);
    validation.set_issuer(allowed_issuers);

    let data = decode::<IdTokenClaims>(token, &key, &validation)
        .map_err(|_| AuthError::InvalidCredentials)?;
    Ok(data.claims)
}

/// Verify a Microsoft ID token from the `common` authority.
///
/// **This is the one provider that cannot reuse [`verify_rs256`]'s fixed issuer
/// list.** Google always issues `accounts.google.com`; Microsoft's `common`
/// authority issues `https://login.microsoftonline.com/{tid}/v2.0`, where `tid`
/// is the signing-in user's *own* tenant — a different value for every
/// organisation, and `9188040d-6c67-4c5b-b112-36a304b66dad` for personal
/// accounts. A constant list would reject every real token; an empty one would
/// accept tokens minted for anybody else's application.
///
/// Microsoft's documented rule is therefore to read the token's own `tid` and
/// require `iss` to equal the authority built from it. That is a check on a
/// claim by another claim, which `jsonwebtoken` cannot express — so issuer
/// validation is turned off there and asserted here, after the signature and
/// audience have already been verified.
pub async fn verify_microsoft_id_token(
    cfg: &MicrosoftAuthConfig,
    id_token: &str,
) -> Result<ProviderIdentity, AuthError> {
    let audiences = allowed_audiences(&cfg.client_ids);
    if audiences.is_empty() {
        return Err(AuthError::ProviderNotConfigured);
    }

    let header = decode_header(id_token).map_err(|_| AuthError::InvalidCredentials)?;
    let kid = header.kid.ok_or(AuthError::InvalidCredentials)?;
    let jwk = find_key(MICROSOFT_JWKS_URL, &MICROSOFT_JWKS_CACHE, &kid).await?;

    let claims = verify_microsoft_claims(&jwk, &audiences, id_token)?;

    // Microsoft does not issue `email_verified` at all. Treating a missing one
    // as verified would let an unverified address link to an existing local
    // account in `sso_sign_in`; the account is Microsoft-controlled either way,
    // but the linking decision is not ours to make on a claim we never got.
    let email = claims
        .email
        .clone()
        .or_else(|| claims.preferred_username.clone())
        .filter(|e| e.contains('@'))
        .ok_or_else(|| {
            AuthError::BadRequest("Microsoft did not return an email address for this account".into())
        })?;

    Ok(ProviderIdentity {
        subject: claims.sub,
        email,
        email_verified: false,
        display_name: claims.name,
    })
}

/// The pure half of [`verify_microsoft_id_token`], split out so the
/// `iss`-matches-`tid` rule can be exercised against a hand-built key.
fn verify_microsoft_claims(
    jwk: &jsonwebtoken::jwk::Jwk,
    allowed_audiences: &[String],
    token: &str,
) -> Result<IdTokenClaims, AuthError> {
    let key = DecodingKey::from_jwk(jwk).map_err(|_| AuthError::InvalidCredentials)?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(allowed_audiences);

    // Deliberately not `set_issuer`: the value depends on a claim in the very
    // token being validated. Checked below instead — never skipped.
    validation.validate_aud = true;
    validation.iss = None;

    let data = decode::<IdTokenClaims>(token, &key, &validation)
        .map_err(|_| AuthError::InvalidCredentials)?;

    let tid = data
        .claims
        .tid
        .as_deref()
        .filter(|t| !t.is_empty())
        .ok_or(AuthError::InvalidCredentials)?;
    let expected = format!("https://login.microsoftonline.com/{tid}/v2.0");
    if data.claims.iss.as_deref() != Some(expected.as_str()) {
        return Err(AuthError::InvalidCredentials);
    }

    Ok(data.claims)
}

pub async fn exchange_microsoft_code(
    cfg: &MicrosoftAuthConfig,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<String, AuthError> {
    let client_id = cfg.desktop_client_id.as_deref().ok_or_else(|| {
        AuthError::BadRequest("Microsoft desktop client is not configured".into())
    })?;

    let mut form = vec![
        ("code", code),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
        ("code_verifier", code_verifier),
        // Without the OIDC scopes echoed here Entra returns an access token
        // and no `id_token`, which reads as a decode failure much later.
        ("scope", "openid email profile offline_access"),
    ];

    // Only when there is one. An app registered under "Mobile and desktop applications" is a
    // **public** client, and Entra refuses a secret from one outright:
    // `AADSTS90023: Public clients can't send a client secret`. PKCE is what secures the
    // exchange there, which is the whole point of RFC 8252 — the secret is not merely optional
    // for such a registration, it is wrong. Google's desktop client is the opposite case and
    // requires one, which is why this is per-provider rather than shared.
    if let Some(secret) = cfg.desktop_client_secret.as_deref().filter(|s| !s.is_empty()) {
        form.push(("client_secret", secret));
    }

    let client = http_client()?;
    let resp = client
        .post(MICROSOFT_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("microsoft token exchange failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("microsoft token exchange rejected ({status}): {body}");
        return Err(AuthError::BadRequest("Microsoft sign-in failed".into()));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        id_token: String,
    }

    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("microsoft token response invalid: {e}")))?;
    Ok(token.id_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::jwk::{CommonParameters, RSAKeyParameters, RSAKeyType};
    use jsonwebtoken::{EncodingKey, Header};
    use serde_json::json;

    #[test]
    fn allowed_audiences_splits_and_trims() {
        let ids = allowed_audiences("abc.apps.googleusercontent.com, def.apps.googleusercontent.com ,,");
        assert_eq!(
            ids,
            vec!["abc.apps.googleusercontent.com", "def.apps.googleusercontent.com"]
        );
    }

    #[test]
    fn allowed_audiences_empty_string_is_empty_vec() {
        assert!(allowed_audiences("").is_empty());
    }

    // A throwaway 2048-bit RSA test keypair (never used for anything real) —
    // generated once with `openssl genrsa` so verify_claims can be exercised
    // against a real signature without a network call to a real provider.
    const TEST_KID: &str = "test-kid";
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCLO/BCoJsAoncn
zWapK93e215+8h6lNVWThYfYzzD7zsTnCU98EzzzvpWMoI/M0ypItZYbfxgGnMeJ
Fdbqb+71vxe0EHmGvodTPeQQqex3S9kPwHrrzUmQ66PS4WSVNeRirdIhRwv2ZMzN
qgTB9NKUhyN9GgxcIy2JeL0/SSIjybOqzPk6/NJvIhS96i0u9tDzOW5BE1Ff99Q7
6KMF6tRUSy610iOZoZQcBG8dmizZ9IG6kIxnW8nvcMwf4ZmMm5Q0pD/x/k5v8HBs
++9gjB2DIfxtksoiDBLnfNBlbio3MzQdzONoSsYsIs1F8/xuTmyW4/zdzE2o+MTv
BxCi4v3rAgMBAAECggEANCdRAVwQg4XXtUiKpl6gnpw0Qr/lu8OFGRug/ZTqh7/1
YIdLxzGbmL+wW/s+sh39Djw6jHUoZj2uvko4dRtVeqbINbIgG1ld7k/WEGneAfee
yHg0cwQ0BL1HYbP8zalgsZfjiISI1hP+5SDE5HQUtv8By5gIvjCNG5vm44A88gNL
qjnVjYLdBSToFzEyIPZdkYf2EwOClnZnXGIOlL24hCV4SXj0gbKxnz3IjN5YwyQG
XVwvLvwOJuOQTtx6Fq1B/LYHC9/NpDyRRBMlaDbbKg+IHKlVgmrFDYNewmFprkjF
3Mfaen8bngRGO3CEP/ydRJk9YIFbTv3G9jz5TMA8IQKBgQDDtvim0vO5Hm3BEth4
MjXbczYkcHhTe5SBpXhcm2Sd5M3PSnxeGPiWJzOZd54hUgupiMoM79KfelDhvnlw
UKKHbpgmIW9N1hKP31l8RPe7AlvHCb1skPX+e8WdnEM5qCskrxE7fu7eq697iSm4
enOmcSfqv1PWhKQ46awK4MeYoQKBgQC2HzS/89QIspc6tP0eO9BfKo66z7UtH1Ll
BPQm9LepP2ZM/OAmiAvoGrqdLBFlTFMCl4+GRQi6opMG1GxZO/rLgMQ3M+fcriMg
YjTpHGvskeFArTfdU65peKhx1TnvW37BCs0e49dtP8O+x3SoQel/BMqdBEWrwSPn
i4hL9Z4PCwKBgQCea+hgWVexnCDpbVDOEo6n4V2NJ4EuylTOkNuZ0qsiaAf0aG29
WWc3W+oXqszUWe5YwAIVcLdEIiWAZcc1FABLskj0bJIFJmiGDxwHTGhe9yzFM2wi
ikClSxkOWGPOMwMhQZioWToQAlccn02nJ2+f5e6SxWaeuWWZMAT0FTlboQKBgGh5
5mznl4+VxCOtiDc74QF3DIImfazw90DiYp2mbWXuNOWde4kfKpVwH/XiPeh6rHQk
NfW0zJkkgmu8mJtoSStNJ0Lzx+NVElmVfPztjQwdc7cCp7WUN83RpfAHfkDNoB1l
8N3znrXRip17FnUfuq9fNEx3EvDAz7QY24uXz6CZAoGAUdgz4yozUKFRdd2FEgZH
7KBJNzI6TN6vxnc99x8aIKxH1EBl5n4OWicgne8GYJJ4n1mwITjsHEfdTpXxJvLq
PbgjKtQj9H5Gi28j2Ijz8HHVqdagBoniKIA/z8gJgqQCoEuUtHQpvDGpKUvDchk2
jOOAImuQv4B3/d5ozBlTQDQ=
-----END PRIVATE KEY-----";
    const TEST_N: &str = "izvwQqCbAKJ3J81mqSvd3ttefvIepTVVk4WH2M8w-87E5wlPfBM8876VjKCPzNMqSLWWG38YBpzHiRXW6m_u9b8XtBB5hr6HUz3kEKnsd0vZD8B6681JkOuj0uFklTXkYq3SIUcL9mTMzaoEwfTSlIcjfRoMXCMtiXi9P0kiI8mzqsz5OvzSbyIUveotLvbQ8zluQRNRX_fUO-ijBerUVEsutdIjmaGUHARvHZos2fSBupCMZ1vJ73DMH-GZjJuUNKQ_8f5Ob_BwbPvvYIwdgyH8bZLKIgwS53zQZW4qNzM0HczjaErGLCLNRfP8bk5sluP83cxNqPjE7wcQouL96w";

    fn test_jwk() -> jsonwebtoken::jwk::Jwk {
        jsonwebtoken::jwk::Jwk {
            common: CommonParameters {
                key_id: Some(TEST_KID.to_string()),
                ..Default::default()
            },
            algorithm: jsonwebtoken::jwk::AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: TEST_N.to_string(),
                e: "AQAB".to_string(),
            }),
        }
    }

    fn sign(claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        let key = EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
        jsonwebtoken::encode(&header, &claims, &key).unwrap()
    }

    fn valid_claims() -> serde_json::Value {
        json!({
            "sub": "user-123",
            "email": "person@example.com",
            "email_verified": true,
            "iss": "https://accounts.google.com",
            "aud": "my-client-id",
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        })
    }

    fn google_audiences() -> Vec<String> {
        vec!["my-client-id".to_string()]
    }

    const GOOGLE_ISSUERS: &[&str] = &["https://accounts.google.com", "accounts.google.com"];

    const MS_TENANT: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";

    fn microsoft_claims() -> serde_json::Value {
        json!({
            "sub": "ms-user-1",
            "email": "person@outlook.com",
            "name": "A Person",
            "tid": MS_TENANT,
            "iss": format!("https://login.microsoftonline.com/{MS_TENANT}/v2.0"),
            "aud": "my-client-id",
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        })
    }

    #[test]
    fn microsoft_accepts_an_issuer_built_from_the_tokens_own_tenant() {
        let token = sign(microsoft_claims());
        let claims = verify_microsoft_claims(&test_jwk(), &google_audiences(), &token).unwrap();
        assert_eq!(claims.sub, "ms-user-1");
        assert_eq!(claims.tid.as_deref(), Some(MS_TENANT));
    }

    /// The whole reason Microsoft does not reuse `verify_claims`. A token signed
    /// by the right key, for the right audience, but whose `iss` names a tenant
    /// other than its own `tid` must be refused — otherwise "any tenant" degrades
    /// into "any issuer that can get a token signed".
    #[test]
    fn microsoft_rejects_an_issuer_that_does_not_match_its_tenant() {
        let mut claims = microsoft_claims();
        claims["iss"] = json!("https://login.microsoftonline.com/some-other-tenant/v2.0");
        let token = sign(claims);
        assert!(verify_microsoft_claims(&test_jwk(), &google_audiences(), &token).is_err());
    }

    #[test]
    fn microsoft_rejects_a_token_with_no_tenant_at_all() {
        let mut claims = microsoft_claims();
        claims["tid"] = json!("");
        let token = sign(claims);
        assert!(verify_microsoft_claims(&test_jwk(), &google_audiences(), &token).is_err());
    }

    #[test]
    fn microsoft_still_checks_the_audience() {
        let mut claims = microsoft_claims();
        claims["aud"] = json!("someone-elses-client-id");
        let token = sign(claims);
        assert!(verify_microsoft_claims(&test_jwk(), &google_audiences(), &token).is_err());
    }

    /// A work account can arrive with no `email` claim; `preferred_username` is
    /// then the only address there is, and dropping it would turn a working
    /// sign-in into "Microsoft did not return an email address".
    #[test]
    fn microsoft_reads_preferred_username_when_there_is_no_email() {
        let mut claims = microsoft_claims();
        claims["email"] = json!(null);
        claims["preferred_username"] = json!("person@contoso.com");
        let token = sign(claims);
        let claims = verify_microsoft_claims(&test_jwk(), &google_audiences(), &token).unwrap();
        assert_eq!(claims.email, None);
        assert_eq!(claims.preferred_username.as_deref(), Some("person@contoso.com"));
    }

    #[test]
    fn verify_claims_accepts_valid_token() {
        let token = sign(valid_claims());
        let claims = verify_claims(&test_jwk(), GOOGLE_ISSUERS, &google_audiences(), &token).unwrap();
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.email.as_deref(), Some("person@example.com"));
        assert!(claims.email_verified);
    }

    #[test]
    fn verify_claims_accepts_string_email_verified() {
        let mut claims = valid_claims();
        claims["email_verified"] = json!("true");
        let token = sign(claims);
        let claims = verify_claims(&test_jwk(), GOOGLE_ISSUERS, &google_audiences(), &token).unwrap();
        assert!(claims.email_verified);
    }

    #[test]
    fn verify_claims_rejects_wrong_audience() {
        let token = sign(valid_claims());
        let wrong_aud = vec!["someone-elses-client-id".to_string()];
        assert!(verify_claims(&test_jwk(), GOOGLE_ISSUERS, &wrong_aud, &token).is_err());
    }

    #[test]
    fn verify_claims_rejects_wrong_issuer() {
        let mut claims = valid_claims();
        claims["iss"] = json!("https://not-google.example.com");
        let token = sign(claims);
        assert!(verify_claims(&test_jwk(), GOOGLE_ISSUERS, &google_audiences(), &token).is_err());
    }

    #[test]
    fn verify_claims_rejects_expired_token() {
        let mut claims = valid_claims();
        claims["exp"] = json!((chrono::Utc::now() - chrono::Duration::hours(1)).timestamp());
        let token = sign(claims);
        assert!(verify_claims(&test_jwk(), GOOGLE_ISSUERS, &google_audiences(), &token).is_err());
    }

    #[test]
    fn verify_claims_rejects_bad_signature() {
        // Sign with the test key but verify against a JWK with a mangled
        // modulus — same shape, wrong key.
        let token = sign(valid_claims());
        let mut bad_jwk = test_jwk();
        if let jsonwebtoken::jwk::AlgorithmParameters::RSA(params) = &mut bad_jwk.algorithm {
            params.n = TEST_N.replacen('i', "j", 1);
        }
        assert!(verify_claims(&bad_jwk, GOOGLE_ISSUERS, &google_audiences(), &token).is_err());
    }
}
