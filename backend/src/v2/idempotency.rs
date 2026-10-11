// SPDX-License-Identifier: AGPL-3.0-or-later
//! `Idempotency-Key` (DESIGN §10; docs/api-v2-plan.md, V9). A write sent with
//! the header answers, when sent again with the same key, what it answered the
//! first time — so a client that lost the answer to "create this" or "pay for
//! that" can retry without doing it twice. Keys are the caller's own, kept 24
//! hours. The same key with a different request is `422
//! idempotency_key_reused`; a retry while the first is still running is `409
//! idempotency_in_progress`. A server error is not kept: retrying it runs again.

use crate::app::AppState;
use crate::auth::middleware::AuthUser;
use crate::v2::problem::Problem;
use axum::body::{Body, to_bytes};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

pub const HEADER: &str = "idempotency-key";
pub const RETENTION_HOURS: i32 = 24;
/// Bodies kept and compared are JSON; anything larger is not a v2 write.
const MAX_BODY: usize = 1024 * 1024;

/// A kept answer: the request's hash, then status, type and body (none yet while running).
type Stored = (String, Option<i16>, Option<String>, Option<Vec<u8>>);

pub async fn middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(key) = request.headers().get(HEADER).and_then(|v| v.to_str().ok()).map(str::to_string) else {
        return next.run(request).await;
    };
    if !matches!(*request.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE) {
        return next.run(request).await;
    }
    if key.is_empty() || key.len() > 255 {
        return Problem::bad_request("invalid_idempotency_key", "Idempotency-Key is 1 to 255 characters").into_response();
    }
    let (mut parts, body) = request.into_parts();
    // Not signed in: no key is kept, and the route answers as it would.
    let Ok(auth) = AuthUser::from_request_parts(&mut parts, &state).await else {
        return next.run(Request::from_parts(parts, body)).await;
    };
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Problem::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large").into_response();
    };
    let mut h = Sha256::new();
    h.update(parts.method.as_str().as_bytes());
    h.update(b"\n");
    h.update(parts.uri.path().as_bytes());
    h.update(b"\n");
    h.update(&bytes);
    let hash = format!("{:x}", h.finalize());
    let pool = state.db();

    let claimed = sqlx::query("INSERT INTO idempotency_keys (user_id, key, request_hash) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
        .bind(auth.user_id)
        .bind(&key)
        .bind(&hash)
        .execute(pool)
        .await;
    match claimed {
        Ok(r) if r.rows_affected() == 1 => {}
        Ok(_) => {
            let row: Option<Stored> = sqlx::query_as(
                "SELECT request_hash, status, content_type, body FROM idempotency_keys
                  WHERE user_id = $1 AND key = $2 AND created_at > now() - make_interval(hours => $3)",
            )
            .bind(auth.user_id)
            .bind(&key)
            .bind(RETENTION_HOURS)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            return match row {
                Some((stored, _, _, _)) if stored != hash => {
                    Problem::new(StatusCode::UNPROCESSABLE_ENTITY, "idempotency_key_reused")
                        .detail("this key was used for a different request")
                        .into_response()
                }
                Some((_, None, _, _)) => Problem::new(StatusCode::CONFLICT, "idempotency_in_progress")
                    .detail("the first request with this key is still being answered")
                    .into_response(),
                Some((_, Some(status), content_type, body)) => {
                    let mut response = (StatusCode::from_u16(status as u16).unwrap_or(StatusCode::OK), body.unwrap_or_default()).into_response();
                    if let Some(ct) = content_type.and_then(|c| HeaderValue::from_str(&c).ok()) {
                        response.headers_mut().insert(header::CONTENT_TYPE, ct);
                    }
                    response.headers_mut().insert("idempotent-replayed", HeaderValue::from_static("true"));
                    response
                }
                // Older than the retention: start afresh under the same key.
                None => {
                    let _ = sqlx::query("UPDATE idempotency_keys SET request_hash = $3, status = NULL, content_type = NULL, body = NULL, created_at = now() WHERE user_id = $1 AND key = $2")
                        .bind(auth.user_id)
                        .bind(&key)
                        .bind(&hash)
                        .execute(pool)
                        .await;
                    run(&state, auth.user_id, &key, next, Request::from_parts(parts, Body::from(bytes))).await
                }
            };
        }
        Err(e) => return Problem::internal(e).into_response(),
    }
    run(&state, auth.user_id, &key, next, Request::from_parts(parts, Body::from(bytes))).await
}

async fn run(state: &AppState, user: uuid::Uuid, key: &str, next: Next, request: Request) -> Response {
    let response = next.run(request).await;
    let pool = state.db();
    if response.status().is_server_error() {
        let _ = sqlx::query("DELETE FROM idempotency_keys WHERE user_id = $1 AND key = $2").bind(user).bind(key).execute(pool).await;
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        let _ = sqlx::query("DELETE FROM idempotency_keys WHERE user_id = $1 AND key = $2").bind(user).bind(key).execute(pool).await;
        return Problem::internal(anyhow::anyhow!("an answer too large to keep for Idempotency-Key")).into_response();
    };
    let content_type = parts.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let _ = sqlx::query("UPDATE idempotency_keys SET status = $3, content_type = $4, body = $5 WHERE user_id = $1 AND key = $2")
        .bind(user)
        .bind(key)
        .bind(parts.status.as_u16() as i16)
        .bind(content_type)
        .bind(bytes.to_vec())
        .execute(pool)
        .await;
    Response::from_parts(parts, Body::from(bytes))
}

/// Keys past their 24 hours, by the daily sweep.
pub async fn prune(pool: &sqlx::PgPool) -> anyhow::Result<u64> {
    Ok(sqlx::query("DELETE FROM idempotency_keys WHERE created_at < now() - make_interval(hours => $1)")
        .bind(RETENTION_HOURS)
        .execute(pool)
        .await?
        .rows_affected())
}
