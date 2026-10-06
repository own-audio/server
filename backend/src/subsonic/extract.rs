// SPDX-License-Identifier: AGPL-3.0-or-later
/// Query extraction that fails the way Subsonic clients expect.
///
/// Two problems with using axum's own extractors directly here, both of which
/// produced plain-text HTTP errors that no Subsonic client can read:
///
/// 1. `axum::extract::Query` is `serde_urlencoded`, which cannot collect a
///    repeated key into a `Vec`. Endpoints taking a repeated `id` (a whole
///    play queue, a multi-song star) were rejected outright.
/// 2. Any rejection — a missing required parameter, a malformed one — became a
///    400 with a plain-text body. The protocol says a missing parameter is
///    error code 10 *inside a normal envelope*, and a client handed
///    unparseable bytes tends to report the server as broken rather than the
///    one call as failed.
///
/// This wraps `axum_extra`'s multi-value `Query` and converts any rejection
/// into the correct envelope, reading `f`/`callback` straight from the query
/// string so the error comes back in the format the caller asked for.
use crate::subsonic::envelope::{self, ResponseFormat, SubsonicErrorCode};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

pub struct SubsonicQuery<T>(pub T);

pub struct SubsonicQueryRejection(Response);

impl IntoResponse for SubsonicQueryRejection {
    fn into_response(self) -> Response {
        self.0
    }
}

impl<T, S> FromRequestParts<S> for SubsonicQuery<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = SubsonicQueryRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum_extra::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum_extra::extract::Query(value)) => Ok(SubsonicQuery(value)),
            Err(_) => {
                let (format, callback) = format_from_query(parts.uri.query().unwrap_or(""));
                Err(SubsonicQueryRejection(envelope::error(
                    format,
                    callback.as_deref(),
                    SubsonicErrorCode::MissingParam,
                    None,
                )))
            }
        }
    }
}

/// Pulls `f` and `callback` out of a raw query string.
///
/// Hand-parsed rather than deserialized because this runs precisely when
/// deserialization has already failed — the response format has to be
/// recoverable from a query string that is, by definition, malformed.
fn format_from_query(query: &str) -> (ResponseFormat, Option<String>) {
    let mut format = None;
    let mut callback = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("f", value)) => format = Some(value.to_string()),
            Some(("callback", value)) => callback = Some(value.to_string()),
            _ => {}
        }
    }
    (ResponseFormat::from_param(format.as_deref()), callback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_format_from_a_query_that_failed_to_parse() {
        let (format, callback) = format_from_query("u=someone&f=json&id=");
        assert_eq!(format, ResponseFormat::Json);
        assert_eq!(callback, None);
    }

    #[test]
    fn defaults_to_xml_when_no_format_is_given() {
        let (format, _) = format_from_query("u=someone");
        assert_eq!(format, ResponseFormat::Xml);
    }

    #[test]
    fn keeps_the_jsonp_callback_so_an_error_is_still_callable() {
        let (format, callback) = format_from_query("f=jsonp&callback=cb7");
        assert_eq!(format, ResponseFormat::Jsonp);
        assert_eq!(callback.as_deref(), Some("cb7"));
    }
}
