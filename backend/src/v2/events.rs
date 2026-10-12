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
//! An `identify` event says one of the caller's identify runs moved on: data
//! `{"run_id", "status", "groups_done", "groups_total"}` (revision 12). Read the
//! run, and its proposals once it is `ready`. It has no id: missed ones are not
//! replayed, so a client reconnecting reads its runs again.
//!
//! Driven by polling the change log of migration 0109 and the runs table, so a
//! change made by a job in another process reaches every API process alike.

use crate::app::AppState;
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Duration;
use uuid::Uuid;

const POLL: Duration = Duration::from_secs(3);

struct Poll {
    pool: sqlx::PgPool,
    /// The change-log position last told.
    last: i64,
    /// Look at once instead of after a pause.
    now: bool,
    /// Each run's status and progress as last told.
    runs: HashMap<Uuid, (String, i32)>,
    queue: VecDeque<Event>,
}

/// Live updates for the caller.
#[utoipa::path(get, path = "/events", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("Last-Event-ID" = Option<i64>, Header, description = "The id of the last event received, to resume")),
    responses((status = 200, description = "`text/event-stream`: `library` events, id = change-log position, data `{\"seq\": n}`; `identify` events (no id), data `{\"run_id\", \"status\", \"groups_done\", \"groups_total\"}`; a comment every 30 s", content_type = "text/event-stream")))]
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
    let initial = Poll { pool, last: start, now: true, runs: HashMap::new(), queue: VecDeque::new() };
    let stream = futures_util::stream::unfold(initial, move |mut p| async move {
        loop {
            if let Some(event) = p.queue.pop_front() {
                return Some((Ok(event), p));
            }
            if !p.now {
                tokio::time::sleep(POLL).await;
            }
            p.now = false;
            let newest: Option<i64> = sqlx::query_scalar(
                "SELECT max(seq) FROM changes WHERE seq > $1 AND (family_id = $2 OR user_id = $3)",
            )
            .bind(p.last)
            .bind(family_id)
            .bind(user_id)
            .fetch_one(&p.pool)
            .await
            .ok()
            .flatten();
            if let Some(seq) = newest {
                p.last = seq;
                p.queue.push_back(Event::default().event("library").id(seq.to_string()).data(format!("{{\"seq\":{seq}}}")));
            }
            // Active runs, and those that ended lately, so the end is told too.
            let runs: Vec<(Uuid, String, i32, i32)> = sqlx::query_as(
                "SELECT id, status, groups_done, groups_total FROM identify_runs
                  WHERE user_id = $1
                    AND (status IN ('queued', 'running', 'waiting') OR finished_at > now() - interval '1 minute')",
            )
            .bind(user_id)
            .fetch_all(&p.pool)
            .await
            .unwrap_or_default();
            for (id, status, done, total) in runs {
                let state = (status.clone(), done);
                if p.runs.get(&id) == Some(&state) {
                    continue;
                }
                p.runs.insert(id, state);
                let data = serde_json::json!({ "run_id": id, "status": status, "groups_done": done, "groups_total": total });
                p.queue.push_back(Event::default().event("identify").data(data.to_string()));
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)).text("keep-alive")))
}
