// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sync for offline clients (DESIGN §8; docs/api-v2-plan.md, V5).
//!
//! - `GET /sync/snapshot` streams everything the caller can see as NDJSON, one
//!   record a line, between a `begin` and an `end` line that carry the cursor
//!   the snapshot was taken at.
//! - `GET /sync/changes` reads the change log after a cursor: each changed
//!   item's current record, or its removal (gone, or no longer visible). A
//!   `reset` means the household's library changed too much to send row by
//!   row: snapshot again. A cursor older than the log is `410
//!   sync_cursor_expired`, which means the same.
//!
//! Records are the shapes of the rest of v2 (`Track`, `Album`, …); a listener's
//! state comes as `favourite.<kind>`, `rating.<kind>`, `progress.<kind>` and
//! `bookmark`, with the record of `GET /me/…`.

use crate::app::AppState;
use crate::auth::middleware::AuthUser;
use crate::db::catalog;
use crate::db::changes::{self, Cursor};
use crate::db::user_state::{self, Kind};
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use crate::v2::{music, spoken, state as st};
use axum::Json;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The kinds a snapshot can hold, in the order it sends them.
pub const SNAPSHOT_KINDS: &[&str] =
    &["contributor", "release_group", "album", "genre", "track", "book", "podcast", "episode", "favourite", "rating", "progress", "bookmark"];

const BATCH: i64 = 500;

fn json_of<T: Serialize>(value: T) -> Result<Value, Problem> {
    serde_json::to_value(value).map_err(|e| Problem::internal(anyhow::anyhow!(e)))
}

/// The current records of these items of one kind, as the caller sees them.
/// An item missing from the answer is gone or not visible.
async fn records(state: &AppState, family: &FamilyContext, auth: &AuthUser, kind: &str, ids: &[Uuid]) -> Result<Vec<(Uuid, Value)>, Problem> {
    let pool = state.db();
    let viewer = family.viewer();
    let mut out = Vec::new();
    match kind {
        "track" => {
            let rows = catalog::tracks_by_ids(pool, viewer, ids).await?;
            for t in music::tracks(state, rows).await? {
                out.push((t.id, json_of(t)?));
            }
        }
        "album" => {
            let rows = catalog::visible_albums_by_ids(pool, viewer, ids).await?;
            for a in music::albums(state, viewer, rows).await? {
                out.push((a.id, json_of(a)?));
            }
        }
        "contributor" => {
            for k in catalog::visible_contributors_by_ids(pool, viewer, ids).await? {
                let c = music::contributor(state, k);
                out.push((c.id, json_of(c)?));
            }
        }
        "release_group" => {
            for g in catalog::visible_release_groups_by_ids(pool, viewer, ids).await? {
                let g = music::release_group(g);
                out.push((g.id, json_of(g)?));
            }
        }
        "genre" => {
            for g in catalog::visible_genres_by_ids(pool, viewer, ids).await? {
                out.push((g.id, json!({ "id": g.id, "name": g.name })));
            }
        }
        "recording" => {
            for id in ids {
                if let Some((r, track_ids)) = catalog::find_recording(pool, viewer, *id).await? {
                    out.push((r.id, json!({ "id": r.id, "title": r.title, "musicbrainz_recording_id": r.musicbrainz_recording_id, "isrc": r.isrc, "track_ids": track_ids })));
                }
            }
        }
        "book" => {
            for row in catalog::books_by_ids(pool, viewer, ids).await? {
                let b = spoken::full_book(state, row).await?;
                out.push((b.id, json_of(b)?));
            }
        }
        "podcast" => {
            let rows = catalog::podcasts_by_ids(pool, viewer, ids).await?;
            let row_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
            let mut credits = spoken::credits_of(state, "podcast", &row_ids).await?;
            for row in rows {
                let c = credits.remove(&row.id).unwrap_or_default();
                let p = spoken::podcast(state, row, c);
                out.push((p.id, json_of(p)?));
            }
        }
        "episode" => {
            for row in catalog::episodes_by_ids(pool, viewer, ids).await? {
                let e = spoken::episode(state, row);
                out.push((e.id, json_of(e)?));
            }
        }
        "bookmark" => {
            for id in ids {
                if let Some(row) = user_state::find_bookmark(pool, family.user_id, *id).await? {
                    out.push((*id, json_of(st::bookmark(state, row).await?)?));
                }
            }
        }
        other => {
            let Some((state_kind, of)) = other.split_once('.') else { return Ok(out) };
            let Some(k) = Kind::parse(of) else { return Ok(out) };
            for id in ids {
                // State about something no longer visible is gone, as far as the client knows.
                if !user_state::is_visible(pool, viewer, k, *id).await? {
                    continue;
                }
                let record = match state_kind {
                    "favourite" if k.can_favourite() => user_state::favourite_at(pool, family.user_id, k, *id)
                        .await?
                        .map(|at| json!({ "kind": of, "id": id, "created_at": at })),
                    "rating" if k.can_rate() => user_state::rating_of(pool, family.user_id, k, *id)
                        .await?
                        .map(|(rating, at)| json!({ "kind": of, "id": id, "rating": rating, "updated_at": at })),
                    "progress" if k.has_progress() => match user_state::get_progress(pool, family.user_id, k, *id).await? {
                        Some(p) => {
                            let mut p = st::progress(k, p);
                            let this = st::device_id(auth);
                            p.devices = Some(
                                user_state::device_positions(pool, family.user_id, k, *id)
                                    .await?
                                    .into_iter()
                                    .map(|d| st::DevicePosition {
                                        current: d.device_id == this,
                                        device_id: d.device_id,
                                        device_name: d.device_name,
                                        device_kind: d.device_kind,
                                        position_ms: d.position_ms,
                                        finished: d.finished,
                                        at: d.at,
                                    })
                                    .collect(),
                            );
                            Some(json_of(p)?)
                        }
                        None => None,
                    },
                    _ => None,
                };
                if let Some(record) = record {
                    out.push((*id, record));
                }
            }
        }
    }
    Ok(out)
}

// ── Changes ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ChangesQuery {
    /// From the snapshot's `end` line or the previous answer; absent to start
    /// from now (with nothing to send yet).
    pub cursor: Option<String>,
    /// Log rows to read, 1 to 1000; 500 when absent.
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct Change {
    /// `track`, `album`, …, `favourite.track`, `progress.book`, `bookmark`.
    pub kind: String,
    pub id: Uuid,
    /// `upsert` (here is the record now) or `delete` (gone, or no longer visible to you).
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub record: Option<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct ChangesAnswer {
    pub changes: Vec<Change>,
    /// For the next call.
    pub cursor: String,
    /// More is waiting: call again at once.
    pub has_more: bool,
    /// The household's library changed too much to send as changes: drop
    /// what you have and take a new snapshot.
    pub reset: bool,
}

/// What changed since the cursor.
#[utoipa::path(get, path = "/sync/changes", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(ChangesQuery),
    responses((status = 200, body = ChangesAnswer),
        (status = 400, description = "`invalid_cursor`", body = crate::v2::problem::ProblemBody),
        (status = 410, description = "`sync_cursor_expired`: take a new snapshot", body = crate::v2::problem::ProblemBody)))]
pub async fn changes(auth: AuthUser, family: FamilyContext, State(state): State<AppState>, Query(q): Query<ChangesQuery>) -> Result<Json<ChangesAnswer>, Problem> {
    let limit = match q.limit {
        None => 500,
        Some(n) if (1..=1000).contains(&n) => n,
        Some(_) => return Err(Problem::bad_request("invalid_limit", "limit is 1 to 1000").field("limit", "out_of_range", "1 to 1000")),
    };
    let Some(raw) = q.cursor.as_deref().filter(|c| !c.trim().is_empty()) else {
        let cursor = changes::now(state.db()).await?;
        return Ok(Json(ChangesAnswer { changes: Vec::new(), cursor: cursor.encode(), has_more: false, reset: false }));
    };
    let cursor = Cursor::decode(raw).ok_or_else(|| Problem::bad_request("invalid_cursor", "the cursor is not one this server handed out").field("cursor", "invalid", "pass it back unchanged"))?;
    if cursor.expired() {
        return Err(Problem::new(StatusCode::GONE, "sync_cursor_expired").detail("the change log no longer reaches back that far: take a new snapshot"));
    }
    let (rows, next, has_more) = changes::read(state.db(), family.viewer(), &cursor, limit).await?;
    if rows.iter().any(|(k, _)| k == "reset") {
        let fresh = changes::now(state.db()).await?;
        return Ok(Json(ChangesAnswer { changes: Vec::new(), cursor: fresh.encode(), has_more: false, reset: true }));
    }
    // One query per kind, the order of the log kept.
    let mut by_kind: BTreeMap<String, Vec<Uuid>> = BTreeMap::new();
    for (k, id) in &rows {
        by_kind.entry(k.clone()).or_default().push(*id);
    }
    let mut found: BTreeMap<(String, Uuid), Value> = BTreeMap::new();
    for (k, ids) in &by_kind {
        for (id, record) in records(&state, &family, &auth, k, ids).await? {
            found.insert((k.clone(), id), record);
        }
    }
    let out = rows
        .into_iter()
        .map(|(kind, id)| match found.remove(&(kind.clone(), id)) {
            Some(record) => Change { kind, id, op: "upsert".into(), record: Some(record) },
            None => Change { kind, id, op: "delete".into(), record: None },
        })
        .collect();
    Ok(Json(ChangesAnswer { changes: out, cursor: next.encode(), has_more, reset: false }))
}

// ── Snapshot ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SnapshotQuery {
    /// Comma-separated kinds; all of them when absent: `contributor`,
    /// `release_group`, `album`, `genre`, `track`, `book`, `podcast`,
    /// `episode`, `favourite`, `rating`, `progress`, `bookmark`.
    pub kinds: Option<String>,
    /// To resume a broken snapshot: the last line's `kind` and `id`, as `kind:id`…
    pub after: Option<String>,
    /// …with the `begin` line's cursor, so the changes after it still follow on.
    pub cursor: Option<String>,
}

/// Everything the caller can see, as NDJSON: a `begin` line with the cursor,
/// one `record` line per item, an `end` line with the same cursor for
/// `GET /sync/changes`.
#[utoipa::path(get, path = "/sync/snapshot", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(SnapshotQuery),
    responses((status = 200, description = "`application/x-ndjson`: `{\"type\":\"begin\",\"cursor\"}`, then `{\"type\":\"record\",\"kind\",\"id\",\"record\"}` lines, then `{\"type\":\"end\",\"cursor\"}`. A body that stops before `end` was cut: resume with `after` and `cursor`.", content_type = "application/x-ndjson"),
        (status = 400, description = "`invalid_kind`, `invalid_cursor`", body = crate::v2::problem::ProblemBody)))]
pub async fn snapshot(auth: AuthUser, family: FamilyContext, State(state): State<AppState>, Query(q): Query<SnapshotQuery>) -> Result<Response, Problem> {
    let kinds: Vec<String> = match q.kinds.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        None => SNAPSHOT_KINDS.iter().map(|k| k.to_string()).collect(),
        Some(list) => {
            let mut wanted = Vec::new();
            for k in list.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                if !SNAPSHOT_KINDS.contains(&k) {
                    return Err(Problem::bad_request("invalid_kind", format!("`{k}` is not in a snapshot")).field("kinds", "invalid", k.to_string()));
                }
                wanted.push(k.to_string());
            }
            // Always in the snapshot's own order, so `after` means the same thing.
            SNAPSHOT_KINDS.iter().filter(|k| wanted.iter().any(|w| w == *k)).map(|k| k.to_string()).collect()
        }
    };
    let cursor = match q.cursor.as_deref().filter(|c| !c.is_empty()) {
        Some(raw) => Cursor::decode(raw).ok_or_else(|| Problem::bad_request("invalid_cursor", "the cursor is not one this server handed out"))?,
        None => changes::now(state.db()).await?,
    };
    let after: Option<(String, Uuid)> = match q.after.as_deref() {
        None | Some("") => None,
        Some(raw) => {
            let parsed = raw.split_once(':').and_then(|(k, id)| Some((k.to_string(), id.parse::<Uuid>().ok()?)));
            Some(parsed.ok_or_else(|| Problem::bad_request("invalid_after", "after is `kind:id` from the last line received"))?)
        }
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(64);
    let line = |v: Value| {
        let mut b = serde_json::to_vec(&v).unwrap_or_default();
        b.push(b'\n');
        Bytes::from(b)
    };
    let encoded = cursor.encode();
    tokio::spawn(async move {
        if tx.send(line(json!({ "type": "begin", "cursor": encoded }))).await.is_err() {
            return;
        }
        let start = after.as_ref().and_then(|(k, _)| kinds.iter().position(|x| x == k)).unwrap_or(0);
        for (i, kind) in kinds.iter().enumerate().skip(start) {
            let mut last = match &after {
                Some((k, id)) if i == start && k == kind => *id,
                _ => Uuid::nil(),
            };
            loop {
                let page = match snapshot_page(&state, &family, &auth, kind, last).await {
                    Ok(page) => page,
                    Err(e) => {
                        tracing::warn!("snapshot of {kind} stopped: {}", e.code());
                        return; // no `end` line: the client resumes
                    }
                };
                let Some(next) = page.last_id else { break };
                for (id, record) in page.records {
                    if tx.send(line(json!({ "type": "record", "kind": wire_kind(kind, &record), "id": id, "record": record }))).await.is_err() {
                        return;
                    }
                }
                last = next;
            }
        }
        let _ = tx.send(line(json!({ "type": "end", "cursor": encoded }))).await;
    });
    let body = Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok::<_, std::io::Error>(b), rx))
    }));
    Ok(([(header::CONTENT_TYPE, "application/x-ndjson")], body).into_response())
}

struct SnapshotPage {
    records: Vec<(Uuid, Value)>,
    /// The last id this page walked past; `None` when the kind is done.
    last_id: Option<Uuid>,
}

/// State lines name what they are about: `favourite.track`, `progress.book`.
fn wire_kind(kind: &str, record: &Value) -> String {
    match kind {
        "favourite" | "rating" | "progress" => format!("{kind}.{}", record.get("kind").and_then(Value::as_str).unwrap_or("")),
        other => other.to_string(),
    }
}

async fn snapshot_page(state: &AppState, family: &FamilyContext, auth: &AuthUser, kind: &str, after: Uuid) -> Result<SnapshotPage, Problem> {
    let pool = state.db();
    let viewer = family.viewer();
    match kind {
        "favourite" | "rating" | "progress" | "bookmark" => {
            // A listener's own state is small: all of it in one page.
            if after != Uuid::nil() {
                return Ok(SnapshotPage { records: Vec::new(), last_id: None });
            }
            let mut records = Vec::new();
            let kinds: &[Kind] = match kind {
                "favourite" => &[Kind::Track, Kind::Album, Kind::Contributor, Kind::Book],
                "rating" => &[Kind::Track, Kind::Album, Kind::Contributor],
                _ => &[Kind::Track, Kind::Book, Kind::Episode],
            };
            for k in kinds {
                let mut after_at = None;
                loop {
                    let ids: Vec<Uuid> = match kind {
                        "favourite" => user_state::list_favourites(pool, viewer, *k, after_at, BATCH).await?.into_iter().map(|(id, at)| {
                            after_at = Some((at, id));
                            id
                        }).collect(),
                        "rating" => user_state::list_ratings(pool, viewer, *k, after_at, BATCH).await?.into_iter().map(|(id, _, at)| {
                            after_at = Some((at, id));
                            id
                        }).collect(),
                        "progress" => user_state::list_progress(pool, viewer, *k, after_at, BATCH).await?.into_iter().map(|p| {
                            after_at = Some((p.updated_at, p.item_id));
                            p.item_id
                        }).collect(),
                        _ => user_state::list_bookmarks(pool, viewer, *k, None, after_at, BATCH).await?.into_iter().map(|b| {
                            after_at = Some((b.created_at, b.id));
                            b.id
                        }).collect(),
                    };
                    let done = (ids.len() as i64) < BATCH;
                    let wire = if kind == "bookmark" { "bookmark".to_string() } else { format!("{kind}.{}", k.as_str()) };
                    records.extend(records_for(state, family, auth, &wire, &ids).await?);
                    if done {
                        break;
                    }
                }
            }
            let last_id = if records.is_empty() { None } else { Some(Uuid::max()) };
            Ok(SnapshotPage { records, last_id })
        }
        _ => {
            let ids = catalog::visible_ids_after(pool, viewer, kind, after, BATCH).await?;
            let last_id = ids.last().copied();
            let mut records = records(state, family, auth, kind, &ids).await?;
            records.sort_by_key(|(id, _)| *id);
            Ok(SnapshotPage { records, last_id })
        }
    }
}

async fn records_for(state: &AppState, family: &FamilyContext, auth: &AuthUser, kind: &str, ids: &[Uuid]) -> Result<Vec<(Uuid, Value)>, Problem> {
    if ids.is_empty() { Ok(Vec::new()) } else { records(state, family, auth, kind, ids).await }
}
