// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET | HEAD | PUT /api/v1/media?k=<key>&e=<expiry>&s=<signature>` — the
//! server's own stand-in for presigned object-store URLs (`storage::links`).
//!
//! Serves local storage, and S3 objects when `STORAGE__PROXY` is set. The
//! signature is the authorisation, exactly as with a presigned URL: no
//! session, so the audio element and native players can follow the link as
//! they follow any other. Range requests are honoured so seeking works, and
//! bodies stream in chunks, so memory does not grow with file size.

use crate::app::AppState;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
pub struct MediaQuery {
    k: String,
    e: i64,
    s: String,
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` (the last n bytes). Multi-range
/// requests are answered with the whole object, which the spec allows.
fn parse_range(value: &str, total_hint: Option<u64>) -> Option<(u64, Option<u64>)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        let total = total_hint?;
        return Some((total.saturating_sub(n), None));
    }
    let start: u64 = a.parse().ok()?;
    let end = if b.is_empty() { None } else { Some(b.parse().ok()?) };
    if end.is_some_and(|e| e < start) {
        return None;
    }
    Some((start, end))
}

pub async fn media(
    State(state): State<AppState>,
    method: Method,
    Query(q): Query<MediaQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let store = state.storage();
    let Some(links) = store.media_links() else {
        // This server hands out presigned store URLs; nothing is served here.
        return status(StatusCode::NOT_FOUND);
    };
    if !links.verify(method.as_str(), &q.k, q.e, &q.s) {
        return status(StatusCode::FORBIDDEN);
    }

    match method {
        Method::PUT => put(&state, &q.k, &headers, body).await,
        Method::GET | Method::HEAD => get(&state, &q.k, &headers, method == Method::HEAD).await,
        _ => status(StatusCode::METHOD_NOT_ALLOWED),
    }
}

async fn get(state: &AppState, key: &str, headers: &HeaderMap, head_only: bool) -> Response {
    let store = state.storage();
    let wanted = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    // A suffix range (`bytes=-n`) needs the size first.
    let total_hint = match wanted {
        Some(v) if v.starts_with("bytes=-") => match store.head_object(key).await {
            Ok(Some((len, _))) => Some(len.max(0) as u64),
            _ => None,
        },
        _ => None,
    };
    let range = wanted.and_then(|v| parse_range(v, total_hint));

    let opened = match store.open(key, range).await {
        Ok(Some(o)) => o,
        // A range past the end of a file that exists is 416; a missing file is 404 either way.
        Ok(None) if range.is_some() && matches!(store.head_object(key).await, Ok(Some(_))) => {
            return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::ACCEPT_RANGES, "bytes")]).into_response();
        }
        Ok(None) => return status(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "media route: open failed");
            return status(StatusCode::BAD_GATEWAY);
        }
    };

    let len = if opened.total == 0 { 0 } else { opened.end - opened.start + 1 };
    let mut res = Response::builder()
        .status(if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, opened.content_type.as_str())
        .header(header::CONTENT_LENGTH, len)
        .header(header::ACCEPT_RANGES, "bytes")
        // The link itself expires; the bytes behind a key never change.
        .header(header::CACHE_CONTROL, "private, max-age=14400");
    if range.is_some() {
        res = res.header(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{}", opened.start, opened.end, opened.total),
        );
    }
    let body = if head_only { Body::empty() } else { opened.body };
    res.body(body).unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
}

async fn put(state: &AppState, key: &str, headers: &HeaderMap, body: Body) -> Response {
    let store = state.storage();
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let temp = match store.new_temp() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "media route: temp file");
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let file = match temp.reopen() {
        Ok(f) => f,
        Err(_) => return status(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mut file = tokio::fs::File::from_std(file);
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return status(StatusCode::BAD_REQUEST);
        };
        if file.write_all(&chunk).await.is_err() {
            return status(StatusCode::INSUFFICIENT_STORAGE);
        }
    }
    if file.flush().await.is_err() {
        return status(StatusCode::INSUFFICIENT_STORAGE);
    }
    drop(file);

    match store.put_temp(key, temp, &content_type).await {
        Ok(()) => {
            let mut res = status(StatusCode::OK);
            res.headers_mut().insert(header::ETAG, HeaderValue::from_static("\"stored\""));
            res
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "media route: store failed");
            status(StatusCode::BAD_GATEWAY)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", None), Some((0, Some(99))));
        assert_eq!(parse_range("bytes=100-", None), Some((100, None)));
        assert_eq!(parse_range("bytes=-10", Some(100)), Some((90, None)));
        assert_eq!(parse_range("bytes=-10", None), None);
        assert_eq!(parse_range("bytes=5-1", None), None);
        assert_eq!(parse_range("bytes=0-1,5-6", None), None);
        assert_eq!(parse_range("items=0-1", None), None);
    }
}
