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
//! `bookmark`, with the record of `GET /me/…`; the household's people as
//! `member` (`MemberRecord`), those who left included.
//!
//! Both take `kinds=` so a client reads only what it keeps, and the snapshot
//! answers ids alone (`fields=ids`) for a client that only checks what it holds
//! is still there.

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
pub const SNAPSHOT_KINDS: &[&str] = &[
    "member", "contributor", "release_group", "album", "genre", "track", "playlist", "smart_playlist", "book", "series", "collection", "podcast",
    "episode", "favourite", "rating", "feedback", "progress", "bookmark",
];

/// The kinds of the change feed: the snapshot's, and recordings (a track's
/// `recording_id`; `GET /recordings/{id}`).
pub const CHANGE_KINDS: &[&str] = &[
    "member", "contributor", "release_group", "album", "genre", "recording", "track", "playlist", "smart_playlist", "book", "series", "collection",
    "podcast", "episode", "favourite", "rating", "feedback", "progress", "bookmark",
];

/// A listener's own state, small enough to send in one go.
const STATE_KINDS: &[&str] = &["favourite", "rating", "feedback", "progress", "bookmark"];

const BATCH: i64 = 500;
/// Ids only are small: more of them a round.
const ID_BATCH: i64 = 5000;

/// `kinds=` as a list of known kinds, or `None` for all of them. `favourite.track` and the
/// like name one kind of a listener's state; `favourite` all of them.
pub(crate) fn parse_kinds(raw: Option<&str>, known: &[&str], dotted: bool) -> Result<Option<Vec<String>>, Problem> {
    let Some(list) = raw.map(str::trim).filter(|k| !k.is_empty()) else { return Ok(None) };
    let mut wanted: Vec<String> = Vec::new();
    for k in list.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        let ok = known.contains(&k)
            || (dotted
                && k.split_once('.').is_some_and(|(state, of)| {
                    Kind::parse(of).is_some_and(|kind| match state {
                        "favourite" => kind.can_favourite(),
                        "rating" => kind.can_rate(),
                        "feedback" => kind == Kind::Track,
                        "progress" => kind.has_progress(),
                        _ => false,
                    })
                }));
        if !ok {
            return Err(Problem::bad_request("invalid_kind", format!("`{k}` is not a kind here")).field("kinds", "invalid", k.to_string()));
        }
        if !wanted.iter().any(|w| w == k) {
            wanted.push(k.to_string());
        }
    }
    Ok(Some(wanted))
}

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
        "member" => {
            for row in crate::db::families::member_records(pool, family.family_id, Some(ids)).await? {
                let m = crate::v2::household::member_record(state, row);
                out.push((m.user_id, json_of(m)?));
            }
        }
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
                let b = spoken::full_book(state, viewer, row).await?;
                out.push((b.id, json_of(b)?));
            }
        }
        "series" => {
            let rows = crate::db::shelves::series_by_ids(pool, viewer, ids).await?;
            for s in crate::v2::shelves::series_records(state, family, rows).await? {
                out.push((s.id, json_of(s)?));
            }
        }
        "playlist" => {
            for row in catalog::playlists_by_ids(pool, viewer, ids).await? {
                let p = crate::v2::playlists::record(state, row);
                out.push((p.id, json_of(p)?));
            }
        }
        "smart_playlist" => {
            for row in crate::db::music::smart_playlists_by_ids(pool, family.user_id, family.family_id, ids).await? {
                let p = crate::v2::smart::record(row);
                out.push((p.id, json_of(p)?));
            }
        }
        "collection" => {
            for c in crate::db::shelves::list_collections(pool, viewer, Some(ids), None, ids.len() as i64).await? {
                let c = crate::v2::shelves::collection(state, c);
                out.push((c.id, json_of(c)?));
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
                    "feedback" if k == Kind::Track => match user_state::feedback_of(pool, family.user_id, *id).await? {
                        Some((stored, at)) => Some(json_of(st::feedback(*id, &stored, at))?),
                        None => None,
                    },
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
    /// Comma-separated kinds to read, every kind when absent: `member`, `contributor`,
    /// `release_group`, `album`, `genre`, `recording`, `track`, `playlist`, `smart_playlist`,
    /// `book`, `series`, `collection`, `podcast`, `episode`, `favourite`, `rating`, `feedback`,
    /// `progress`, `bookmark`, or one kind of state (`progress.episode`, `feedback.track`). Filtered by the server, so the other kinds cost nothing; keep
    /// passing the same list with the cursor. A `reset` always comes through.
    pub kinds: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Change {
    /// `track`, `album`, …, `member`, `series`, `collection`, `playlist`, `smart_playlist`,
    /// `favourite.track`, `feedback.track`, `progress.book`, `bookmark`. A client skips kinds it
    /// does not know.
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
        (status = 400, description = "`invalid_cursor`, `invalid_kind`, `invalid_limit`", body = crate::v2::problem::ProblemBody),
        (status = 410, description = "`sync_cursor_expired`: take a new snapshot", body = crate::v2::problem::ProblemBody)))]
pub async fn changes(auth: AuthUser, family: FamilyContext, State(state): State<AppState>, Query(q): Query<ChangesQuery>) -> Result<Json<ChangesAnswer>, Problem> {
    let limit = match q.limit {
        None => 500,
        Some(n) if (1..=1000).contains(&n) => n,
        Some(_) => return Err(Problem::bad_request("invalid_limit", "limit is 1 to 1000").field("limit", "out_of_range", "1 to 1000")),
    };
    let kinds = parse_kinds(q.kinds.as_deref(), CHANGE_KINDS, true)?;
    let Some(raw) = q.cursor.as_deref().filter(|c| !c.trim().is_empty()) else {
        let cursor = changes::now(state.db()).await?;
        return Ok(Json(ChangesAnswer { changes: Vec::new(), cursor: cursor.encode(), has_more: false, reset: false }));
    };
    let cursor = Cursor::decode(raw).ok_or_else(|| Problem::bad_request("invalid_cursor", "the cursor is not one this server handed out").field("cursor", "invalid", "pass it back unchanged"))?;
    if cursor.expired() {
        return Err(Problem::new(StatusCode::GONE, "sync_cursor_expired").detail("the change log no longer reaches back that far: take a new snapshot"));
    }
    let (rows, next, has_more) = changes::read(state.db(), family.viewer(), &cursor, kinds.as_deref(), limit).await?;
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
    /// Comma-separated kinds; all of them when absent: `member`, `contributor`,
    /// `release_group`, `album`, `genre`, `track`, `playlist`, `smart_playlist`, `book`,
    /// `series`, `collection`, `podcast`, `episode`, `favourite`, `rating`, `feedback`,
    /// `progress`, `bookmark`.
    pub kinds: Option<String>,
    /// `ids`: each line names a record (`{"type":"id","kind","id"}`) without it, for a client
    /// that checks what it holds is still there; about 60 bytes a line instead of a record.
    pub fields: Option<String>,
    /// To resume a broken snapshot: the last line's `kind` and `id`, as `kind:id`…
    pub after: Option<String>,
    /// …with the `begin` line's cursor, so the changes after it still follow on.
    pub cursor: Option<String>,
}

/// Everything the caller can see, as NDJSON: a `begin` line with the cursor,
/// one `record` line per item (or one `id` line with `fields=ids`), an `end`
/// line with the same cursor for `GET /sync/changes`.
#[utoipa::path(get, path = "/sync/snapshot", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(SnapshotQuery),
    responses((status = 200, description = "`application/x-ndjson`: `{\"type\":\"begin\",\"cursor\"}`, then `{\"type\":\"record\",\"kind\",\"id\",\"record\"}` lines (`{\"type\":\"id\",\"kind\",\"id\"}` with `fields=ids`), then `{\"type\":\"end\",\"cursor\"}`. A body that stops before `end` was cut: resume with `after` and `cursor`.", content_type = "application/x-ndjson"),
        (status = 400, description = "`invalid_kind`, `invalid_fields`, `invalid_cursor`, `invalid_after`", body = crate::v2::problem::ProblemBody)))]
pub async fn snapshot(auth: AuthUser, family: FamilyContext, State(state): State<AppState>, Query(q): Query<SnapshotQuery>) -> Result<Response, Problem> {
    let kinds: Vec<String> = match parse_kinds(q.kinds.as_deref(), SNAPSHOT_KINDS, false)? {
        None => SNAPSHOT_KINDS.iter().map(|k| k.to_string()).collect(),
        // Always in the snapshot's own order, so `after` means the same thing.
        Some(wanted) => SNAPSHOT_KINDS.iter().filter(|k| wanted.iter().any(|w| w == *k)).map(|k| k.to_string()).collect(),
    };
    let ids_only = match q.fields.as_deref().map(str::trim) {
        None | Some("") => false,
        Some("ids") => true,
        Some(other) => {
            return Err(Problem::bad_request("invalid_fields", format!("fields is `ids` or absent, not `{other}`")).field("fields", "invalid", other.to_string()));
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
        // A state line names its kind as `favourite.track`: resume in `favourite`.
        let base = |k: &str| k.split('.').next().unwrap_or(k).to_string();
        let start = after.as_ref().and_then(|(k, _)| kinds.iter().position(|x| *x == base(k))).unwrap_or(0);
        for (i, kind) in kinds.iter().enumerate().skip(start) {
            let mut last = match &after {
                // The household's people and a listener's state come whole: sent again.
                Some((k, id)) if i == start && base(k) == *kind && !is_whole(kind) => *id,
                _ => Uuid::nil(),
            };
            loop {
                if ids_only {
                    let page = match id_page(&state, &family, kind, last).await {
                        Ok(page) => page,
                        Err(e) => {
                            tracing::warn!("snapshot ids of {kind} stopped: {}", e.code());
                            return;
                        }
                    };
                    let Some(next) = page.last_id else { break };
                    for (wire, id) in page.ids {
                        if tx.send(line(json!({ "type": "id", "kind": wire, "id": id }))).await.is_err() {
                            return;
                        }
                    }
                    last = next;
                    continue;
                }
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
    // Fused: the compression layer polls once more after the end, and a bare
    // `unfold` panics on that (see `http::json_stream`).
    let body = Body::from_stream(futures_util::StreamExt::fuse(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok::<_, std::io::Error>(b), rx))
    })));
    Ok(([(header::CONTENT_TYPE, "application/x-ndjson")], body).into_response())
}

fn is_whole(kind: &str) -> bool {
    kind == "member" || STATE_KINDS.contains(&kind)
}

struct SnapshotPage {
    records: Vec<(Uuid, Value)>,
    /// The last id this page walked past; `None` when the kind is done.
    last_id: Option<Uuid>,
}

/// State lines name what they are about: `favourite.track`, `progress.book`.
fn wire_kind(kind: &str, record: &Value) -> String {
    match kind {
        "favourite" | "rating" | "feedback" | "progress" => format!("{kind}.{}", record.get("kind").and_then(Value::as_str).unwrap_or("")),
        other => other.to_string(),
    }
}

async fn snapshot_page(state: &AppState, family: &FamilyContext, auth: &AuthUser, kind: &str, after: Uuid) -> Result<SnapshotPage, Problem> {
    let pool = state.db();
    let viewer = family.viewer();
    match kind {
        "member" => {
            if after != Uuid::nil() {
                return Ok(SnapshotPage { records: Vec::new(), last_id: None });
            }
            let records = crate::db::families::member_records(pool, family.family_id, None)
                .await?
                .into_iter()
                .map(|row| {
                    let m = crate::v2::household::member_record(state, row);
                    Ok((m.user_id, json_of(m)?))
                })
                .collect::<Result<Vec<_>, Problem>>()?;
            let last_id = if records.is_empty() { None } else { Some(Uuid::max()) };
            Ok(SnapshotPage { records, last_id })
        }
        "favourite" | "rating" | "feedback" | "progress" | "bookmark" => {
            // A listener's own state is small: all of it in one page.
            if after != Uuid::nil() {
                return Ok(SnapshotPage { records: Vec::new(), last_id: None });
            }
            let mut records = Vec::new();
            let kinds: &[Kind] = match kind {
                "favourite" => &[Kind::Track, Kind::Album, Kind::Contributor, Kind::Book],
                "rating" => &[Kind::Track, Kind::Album, Kind::Contributor],
                "feedback" => &[Kind::Track],
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
                        "feedback" => user_state::list_feedback(pool, viewer, None, after_at, BATCH).await?.into_iter().map(|(id, _, at)| {
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

struct IdPage {
    /// (the line's kind, id).
    ids: Vec<(String, Uuid)>,
    last_id: Option<Uuid>,
}

/// The ids a snapshot of `kind` would send, without building a record.
async fn id_page(state: &AppState, family: &FamilyContext, kind: &str, after: Uuid) -> Result<IdPage, Problem> {
    let pool = state.db();
    let viewer = family.viewer();
    if !is_whole(kind) {
        let ids = catalog::visible_ids_after(pool, viewer, kind, after, ID_BATCH).await?;
        let last_id = ids.last().copied();
        return Ok(IdPage { ids: ids.into_iter().map(|id| (kind.to_string(), id)).collect(), last_id });
    }
    if after != Uuid::nil() {
        return Ok(IdPage { ids: Vec::new(), last_id: None });
    }
    let mut ids: Vec<(String, Uuid)> = Vec::new();
    if kind == "member" {
        ids = crate::db::families::member_records(pool, family.family_id, None).await?.into_iter().map(|m| ("member".to_string(), m.user_id)).collect();
    } else {
        let kinds: &[Kind] = match kind {
            "favourite" => &[Kind::Track, Kind::Album, Kind::Contributor, Kind::Book],
            "rating" => &[Kind::Track, Kind::Album, Kind::Contributor],
            "feedback" => &[Kind::Track],
            _ => &[Kind::Track, Kind::Book, Kind::Episode],
        };
        for k in kinds {
            let wire = if kind == "bookmark" { "bookmark".to_string() } else { format!("{kind}.{}", k.as_str()) };
            let mut after_at = None;
            loop {
                let page: Vec<Uuid> = match kind {
                    "favourite" => user_state::list_favourites(pool, viewer, *k, after_at, ID_BATCH).await?.into_iter().map(|(id, at)| {
                        after_at = Some((at, id));
                        id
                    }).collect(),
                    "rating" => user_state::list_ratings(pool, viewer, *k, after_at, ID_BATCH).await?.into_iter().map(|(id, _, at)| {
                        after_at = Some((at, id));
                        id
                    }).collect(),
                    "feedback" => user_state::list_feedback(pool, viewer, None, after_at, ID_BATCH).await?.into_iter().map(|(id, _, at)| {
                        after_at = Some((at, id));
                        id
                    }).collect(),
                    "progress" => user_state::list_progress(pool, viewer, *k, after_at, ID_BATCH).await?.into_iter().map(|p| {
                        after_at = Some((p.updated_at, p.item_id));
                        p.item_id
                    }).collect(),
                    _ => user_state::list_bookmarks(pool, viewer, *k, None, after_at, ID_BATCH).await?.into_iter().map(|b| {
                        after_at = Some((b.created_at, b.id));
                        b.id
                    }).collect(),
                };
                let done = (page.len() as i64) < ID_BATCH;
                ids.extend(page.into_iter().map(|id| (wire.clone(), id)));
                if done {
                    break;
                }
            }
        }
    }
    let last_id = if ids.is_empty() { None } else { Some(Uuid::max()) };
    Ok(IdPage { ids, last_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(p: Problem) -> String {
        p.code().to_string()
    }

    #[test]
    fn kinds_are_checked_and_kept_once() {
        assert_eq!(parse_kinds(None, CHANGE_KINDS, true).unwrap(), None);
        assert_eq!(parse_kinds(Some(" "), CHANGE_KINDS, true).unwrap(), None);
        assert_eq!(
            parse_kinds(Some("track, episode,track"), CHANGE_KINDS, true).unwrap(),
            Some(vec!["track".to_string(), "episode".to_string()])
        );
        assert_eq!(parse_kinds(Some("progress.episode,member"), CHANGE_KINDS, true).unwrap(), Some(vec!["progress.episode".into(), "member".into()]));
        assert_eq!(code(parse_kinds(Some("track,teleport"), CHANGE_KINDS, true).unwrap_err()), "invalid_kind");
        // A kind of state that does not exist, and dotted kinds where they are not taken.
        assert_eq!(code(parse_kinds(Some("rating.book"), CHANGE_KINDS, true).unwrap_err()), "invalid_kind");
        assert_eq!(code(parse_kinds(Some("favourite.episode"), CHANGE_KINDS, true).unwrap_err()), "invalid_kind");
        assert_eq!(code(parse_kinds(Some("progress.book"), SNAPSHOT_KINDS, false).unwrap_err()), "invalid_kind");
        assert_eq!(code(parse_kinds(Some("recording"), SNAPSHOT_KINDS, false).unwrap_err()), "invalid_kind");
        assert!(parse_kinds(Some("recording"), CHANGE_KINDS, true).is_ok());
        // Playlists and a listener's feedback (phase 6).
        assert!(parse_kinds(Some("playlist,smart_playlist,feedback,feedback.track"), CHANGE_KINDS, true).is_ok());
        assert!(parse_kinds(Some("playlist,smart_playlist,feedback"), SNAPSHOT_KINDS, false).is_ok());
        assert_eq!(code(parse_kinds(Some("feedback.book"), CHANGE_KINDS, true).unwrap_err()), "invalid_kind");
    }

    #[test]
    fn every_snapshot_kind_is_a_change_kind() {
        for k in SNAPSHOT_KINDS {
            assert!(CHANGE_KINDS.contains(k), "{k}");
        }
        assert_eq!(SNAPSHOT_KINDS[0], "member", "owners' names come before what they own");
    }

    #[test]
    fn small_kinds_are_sent_whole() {
        for k in ["member", "favourite", "rating", "feedback", "progress", "bookmark"] {
            assert!(is_whole(k), "{k}");
        }
        for k in ["track", "book", "episode", "album", "contributor", "playlist", "smart_playlist"] {
            assert!(!is_whole(k), "{k}");
        }
    }

    #[test]
    fn state_lines_name_their_kind() {
        assert_eq!(wire_kind("favourite", &json!({"kind": "track"})), "favourite.track");
        assert_eq!(wire_kind("member", &json!({"user_id": "x"})), "member");
        assert_eq!(wire_kind("feedback", &json!({"kind": "track"})), "feedback.track");
    }
}
