// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live updates (DESIGN §10; docs/api-v2-plan.md, V8): `GET /events`, as
//! Server-Sent Events.
//!
//! A `library` event says the caller's household or the caller's own state
//! changed: read `GET /sync/changes` with the cursor you hold. The events
//! carry no records — the feed is the one place changes come from, so a
//! client never has two paths to keep consistent. Each event's id is the
//! change log's position; reconnecting with `Last-Event-ID` picks up from
//! there. A comment every 30 seconds keeps proxies from closing the stream.
//!
//! Driven by polling the change log of migration 0109, so a change made by a
//! job in another process reaches every API process alike.

use crate::app::AppState;
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use std::convert::Infallible;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(3);

/// Live updates for the caller.
#[utoipa::path(get, path = "/events", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("Last-Event-ID" = Option<i64>, Header, description = "The id of the last event received, to resume")),
    responses((status = 200, description = "`text/event-stream`: `library` events, id = change-log position, data `{\"seq\": n}`; a comment every 30 s", content_type = "text/event-stream")))]
pub async fn events(
    family: FamilyContext,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Problem> {
    let resume = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let pool = state.db().clone();
    let start = match resume {
        Some(seq) => seq,
        None => sqlx::query_scalar::<_, Option<i64>>("SELECT max(seq) FROM changes")
            .fetch_one(&pool)
            .await
            .map_err(anyhow::Error::from)?
            .unwrap_or(0),
    };
    let (family_id, user_id) = (family.family_id, family.user_id);
    let stream = futures_util::stream::unfold((pool, start, resume.is_some()), move |(pool, last, mut now)| async move {
        loop {
            if !now {
                tokio::time::sleep(POLL).await;
            }
            now = false;
            let newest: Option<i64> = sqlx::query_scalar(
                "SELECT max(seq) FROM changes WHERE seq > $1 AND (family_id = $2 OR user_id = $3)",
            )
            .bind(last)
            .bind(family_id)
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .ok()
            .flatten();
            if let Some(seq) = newest {
                let event = Event::default().event("library").id(seq.to_string()).data(format!("{{\"seq\":{seq}}}"));
                return Some((Ok(event), (pool, seq, false)));
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)).text("keep-alive")))
}
