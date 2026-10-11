// SPDX-License-Identifier: AGPL-3.0-or-later
/// Subsonic protocol auth: `u` + either a token (`t`+`s`) or a legacy
/// plaintext/`enc:`-hex password (`p`), verified against a per-user
/// Subsonic API key (see `db::subsonic`) — never the real account password,
/// which is argon2-hashed and therefore cannot be reproduced client-side.
use crate::app::AppState;
use crate::db;
use crate::subsonic::envelope::{self, ResponseFormat, SubsonicErrorCode};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use md5::{Digest, Md5};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubsonicAuthUser {
    pub user_id: Uuid,
    pub family_id: Uuid,
    pub is_family_admin: bool,
    /// The name the client authenticated with — audio2 has no separate
    /// username, so this is the account's email, and it is what `getUser`
    /// must echo back for a client to match the response to its own login.
    pub username: String,
    pub format: ResponseFormat,
    pub jsonp_callback: Option<String>,
}

impl SubsonicAuthUser {
    /// Subsonic clients see exactly what the web UI shows this user: their
    /// own content plus family-shared content they are granted.
    pub fn viewer(&self) -> crate::db::access::Viewer {
        crate::db::access::Viewer {
            user_id: self.user_id,
            family_id: self.family_id,
            is_family_admin: self.is_family_admin,
        }
    }
}

pub struct SubsonicRejection(Response);

impl IntoResponse for SubsonicRejection {
    fn into_response(self) -> Response {
        self.0
    }
}

#[derive(Debug, Default)]
struct RawParams {
    u: Option<String>,
    p: Option<String>,
    t: Option<String>,
    s: Option<String>,
    f: Option<String>,
    callback: Option<String>,
}

impl RawParams {
    /// Hand-parsed rather than deserialized, because deserializing cannot fail
    /// *safely* here.
    ///
    /// Both `serde_urlencoded` (axum's `Query`) and `serde_html_form`
    /// (`axum_extra`'s) reject the entire query string when a scalar field
    /// appears twice. There is nothing to fall back to at that point — an
    /// all-empty `RawParams` reports error 10 "Required parameter is missing"
    /// for a request that carried full credentials, and loses `f` along with
    /// them, so the refusal comes back as XML to a client that asked for JSON.
    ///
    /// A repeated parameter is a client quirk, not an attack, and the
    /// long-standing convention for one is last-wins. Unknown keys are ignored
    /// rather than being an error, which is what lets clients append their own.
    fn from_query(query: &str) -> Self {
        let mut params = RawParams::default();
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            let slot = match key.as_ref() {
                "u" => &mut params.u,
                "p" => &mut params.p,
                "t" => &mut params.t,
                "s" => &mut params.s,
                "f" => &mut params.f,
                "callback" => &mut params.callback,
                _ => continue,
            };
            *slot = Some(value.into_owned());
        }
        params
    }
}

impl FromRequestParts<AppState> for SubsonicAuthUser {
    type Rejection = SubsonicRejection;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let params = RawParams::from_query(parts.uri.query().unwrap_or(""));

        let format = ResponseFormat::from_param(params.f.as_deref());
        let callback = params.callback.clone();

        let reject = |code: SubsonicErrorCode| {
            SubsonicRejection(envelope::error(format, callback.as_deref(), code, None))
        };

        let Some(username) = params.u.as_deref().filter(|s| !s.is_empty()) else {
            return Err(reject(SubsonicErrorCode::MissingParam));
        };

        // Case- and whitespace-insensitive: `u` is typed into a phone, where
        // the keyboard capitalises the first letter by default.
        let user = db::users::find_by_email_ci(state.db(), username)
            .await
            .map_err(|_| reject(SubsonicErrorCode::Generic))?
            .ok_or_else(|| reject(SubsonicErrorCode::WrongCredentials))?;

        if !user.is_active {
            return Err(reject(SubsonicErrorCode::NotAuthorized));
        }

        let api_key = db::subsonic::get_key(state.db(), state.at_rest(), user.id)
            .await
            .map_err(|_| reject(SubsonicErrorCode::Generic))?
            .ok_or_else(|| reject(SubsonicErrorCode::WrongCredentials))?;

        let authenticated = match (&params.t, &params.s) {
            (Some(token), Some(salt)) => {
                let mut hasher = Md5::new();
                hasher.update(api_key.as_bytes());
                hasher.update(salt.as_bytes());
                to_hex(&hasher.finalize()).eq_ignore_ascii_case(token)
            }
            // The salted-token form cannot be repaired server-side: the client
            // hashed whatever it held, so a key pasted with a stray space is
            // unrecoverable here. The plaintext form can be, and is — a key
            // copied off a settings page very often arrives with trailing
            // whitespace attached.
            _ => match params.p.as_deref() {
                Some(raw) => decode_password(raw.trim())
                    .map(|pw| pw == api_key)
                    .unwrap_or(false),
                None => false,
            },
        };

        if !authenticated {
            return Err(reject(SubsonicErrorCode::WrongCredentials));
        }

        if crate::demo::is_read_only_account(state, &user.email) && crate::demo::subsonic_refuses(parts.uri.path()) {
            return Err(reject(SubsonicErrorCode::NotAuthorized));
        }

        let membership = db::families::ensure_membership(state.db(), user.id, state.hooks().one_family())
            .await
            .map_err(|_| reject(SubsonicErrorCode::Generic))?;

        Ok(SubsonicAuthUser {
            user_id: user.id,
            family_id: membership.family_id,
            is_family_admin: membership.role == "family_admin" || user.role == "admin",
            username: user.email,
            format,
            jsonp_callback: callback,
        })
    }
}

fn decode_password(raw: &str) -> Option<String> {
    match raw.strip_prefix("enc:") {
        Some(hex) => decode_hex(hex),
        None => Some(raw.to_string()),
    }
}

fn decode_hex(hex: &str) -> Option<String> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let chars: Vec<char> = hex.chars().collect();
    for pair in chars.chunks(2) {
        let byte = u8::from_str_radix(&pair.iter().collect::<String>(), 16).ok()?;
        bytes.push(byte);
    }
    String::from_utf8(bytes).ok()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_credential_parameters() {
        let p = RawParams::from_query("u=someone%40example.com&t=abc&s=xyz&f=json");
        assert_eq!(p.u.as_deref(), Some("someone@example.com"));
        assert_eq!(p.t.as_deref(), Some("abc"));
        assert_eq!(p.s.as_deref(), Some("xyz"));
        assert_eq!(p.f.as_deref(), Some("json"));
    }

    /// The whole reason this is hand-parsed: serde rejects the request outright,
    /// which reported a missing parameter for a fully-credentialed call.
    #[test]
    fn a_repeated_parameter_takes_the_last_value_rather_than_failing() {
        let p = RawParams::from_query("u=first&u=second&t=abc&s=xyz");
        assert_eq!(p.u.as_deref(), Some("second"));
        assert_eq!(p.t.as_deref(), Some("abc"));
    }

    #[test]
    fn unknown_parameters_are_ignored_so_clients_may_add_their_own() {
        let p = RawParams::from_query("u=someone&c=Symfonium&v=1.16.1&extra=1");
        assert_eq!(p.u.as_deref(), Some("someone"));
    }

    #[test]
    fn an_empty_query_yields_no_credentials_rather_than_panicking() {
        let p = RawParams::from_query("");
        assert!(p.u.is_none() && p.p.is_none() && p.t.is_none());
    }

    /// A key pasted on a phone routinely arrives with whitespace attached; the
    /// salted-token form cannot be repaired, but the plaintext form can.
    #[test]
    fn a_plaintext_password_decodes_after_trimming() {
        assert_eq!(decode_password("secret".trim()), Some("secret".to_string()));
        assert_eq!(decode_password(" secret \n".trim()), Some("secret".to_string()));
    }

    #[test]
    fn the_hex_encoded_password_form_round_trips() {
        assert_eq!(decode_password("enc:736563726574"), Some("secret".to_string()));
        assert_eq!(decode_password("enc:not-hex"), None);
    }
}
