// SPDX-License-Identifier: AGPL-3.0-or-later
//! Listening statistics (docs/api-v2-completion-plan.md, phase 9): a listener's totals over a
//! range, grouped by day, week or month; the plays they recorded, page by page; and the
//! household's figures for its admins, of the members who share them.
//!
//! Read from the plays themselves (`listening_sessions`, what `POST /me/plays` records), as v1
//! does: the hourly rollup counts UTC days, and a listener's day starts at their own midnight.
//! Listening history is personal: a member's figures reach the household's admins only when the
//! member shares them (`stats_visible_to_admin` in `/me/settings`), or when an admin turns that on
//! for an account they manage (v1's rule, decision D2 of the family plan).

use crate::app::AppState;
use crate::db;
use crate::families::{FamilyAdmin, FamilyContext};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The longest range one question may cover.
const MAX_RANGE_DAYS: i64 = 3660;

/// v2's item kinds to the stored media kinds.
fn media_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "track" => Some(db::access::MUSIC),
        "book" => Some(db::access::AUDIOBOOK),
        "episode" => Some(db::access::PODCAST),
        _ => None,
    }
}

/// The stored media kind in v2's words: what was listened to.
fn item_kind(media: &str) -> &'static str {
    match media {
        db::access::AUDIOBOOK => "book",
        db::access::PODCAST => "episode",
        _ => "track",
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatsQuery {
    /// Start of the range, RFC 3339; 30 days before `to` when absent.
    pub from: Option<DateTime<Utc>>,
    /// End of the range (not included), RFC 3339; now when absent. At most ten years after `from`.
    pub to: Option<DateTime<Utc>>,
    /// `day` (the default), `week` (from Monday) or `month`, in the listener's local time.
    pub group: Option<String>,
    /// Comma-separated `track`, `book`, `episode`; all of them when absent.
    pub kind: Option<String>,
    /// Minutes east of UTC, so a day starts at the listener's midnight; −720 to 840, 0 when
    /// absent.
    pub tz_offset_minutes: Option<i32>,
}

/// The question, checked.
#[derive(Debug, PartialEq)]
pub(crate) struct Range {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub group: &'static str,
    /// Stored media kinds; `None` for all.
    pub kinds: Option<Vec<String>>,
    pub tz: i32,
}

pub(crate) fn range(q: &StatsQuery, now: DateTime<Utc>) -> Result<Range, Problem> {
    let to = q.to.unwrap_or(now);
    let from = q.from.unwrap_or(to - Duration::days(30));
    if from >= to {
        return Err(Problem::bad_request("invalid_range", "from is before to").field("from", "invalid", "before `to`"));
    }
    if to - from > Duration::days(MAX_RANGE_DAYS) {
        return Err(Problem::bad_request("invalid_range", "a range is at most ten years").field("from", "out_of_range", "at most ten years before `to`"));
    }
    let group = match q.group.as_deref().map(str::trim) {
        None | Some("") | Some("day") => "day",
        Some("week") => "week",
        Some("month") => "month",
        Some(other) => {
            return Err(Problem::bad_request("invalid_group", format!("`{other}` is not day, week or month")).field("group", "invalid", "day, week or month"));
        }
    };
    let kinds = match q.kind.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        None => None,
        Some(list) => {
            let mut out: Vec<String> = Vec::new();
            for k in list.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                let m = media_kind(k).ok_or_else(|| Problem::bad_request("unsupported_kind", format!("`{k}` has no statistics")).field("kind", "invalid", "track, book or episode"))?;
                if !out.iter().any(|x| x == m) {
                    out.push(m.to_string());
                }
            }
            Some(out)
        }
    };
    let tz = q.tz_offset_minutes.unwrap_or(0);
    if !(-720..=840).contains(&tz) {
        return Err(Problem::bad_request("invalid_tz", "tz_offset_minutes is -720 to 840").field("tz_offset_minutes", "out_of_range", "-720 to 840"));
    }
    Ok(Range { from, to, group, kinds, tz })
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct KindTotal {
    /// `track`, `book` or `episode`.
    pub kind: String,
    pub listened_ms: i64,
    pub plays: i64,
}

#[derive(Serialize, ToSchema)]
pub struct Bucket {
    /// The bucket's first day, `YYYY-MM-DD` in the listener's time (a Monday for weeks, the 1st
    /// for months). Buckets without listening are left out.
    pub start: String,
    pub listened_ms: i64,
    pub plays: i64,
    pub by_kind: Vec<KindTotal>,
}

#[derive(Serialize, ToSchema)]
pub struct TopItem {
    /// `track`, `book` or `episode`.
    pub kind: String,
    pub id: Uuid,
    pub listened_ms: i64,
    pub plays: i64,
}

#[derive(Serialize, ToSchema)]
pub struct Stats {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub group: String,
    pub listened_ms: i64,
    pub plays: i64,
    pub by_kind: Vec<KindTotal>,
    pub buckets: Vec<Bucket>,
    /// The ten items listened to longest in the range. Records come from sync or the item's
    /// own read; one no longer there is still named.
    pub top: Vec<TopItem>,
    /// Days in a row, up to today, with any listening (any kind, whatever the range).
    pub streak_days: i64,
    /// Books finished, ever.
    pub finished_books: i64,
}

fn by_kind(rows: &[db::stats::KindTotal]) -> Vec<KindTotal> {
    rows.iter()
        .map(|k| KindTotal { kind: item_kind(&k.media_kind).into(), listened_ms: k.seconds_listened * 1000, plays: k.session_count })
        .collect()
}

/// The buckets from (bucket start, media kind, seconds, plays) rows in bucket order.
fn buckets(rows: Vec<(chrono::NaiveDate, String, i64, i64)>) -> Vec<Bucket> {
    let mut out: Vec<Bucket> = Vec::new();
    for (day, media, secs, plays) in rows {
        let start = day.to_string();
        if out.last().is_none_or(|b| b.start != start) {
            out.push(Bucket { start, listened_ms: 0, plays: 0, by_kind: Vec::new() });
        }
        if let Some(b) = out.last_mut() {
            b.listened_ms += secs * 1000;
            b.plays += plays;
            b.by_kind.push(KindTotal { kind: item_kind(&media).into(), listened_ms: secs * 1000, plays });
        }
    }
    out
}

/// The caller's listening over a range: totals, buckets, the most listened, the streak.
#[utoipa::path(get, path = "/me/stats", tag = "stats", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(StatsQuery),
    responses((status = 200, body = Stats),
        (status = 400, description = "`invalid_range`, `invalid_group`, `unsupported_kind`, `invalid_tz`", body = crate::v2::problem::ProblemBody)))]
pub async fn my_stats(family: FamilyContext, State(state): State<AppState>, Query(q): Query<StatsQuery>) -> Result<Json<Stats>, Problem> {
    let r = range(&q, Utc::now())?;
    let pool = state.db();
    let kinds = r.kinds.as_deref();
    let totals = db::stats::totals_between(pool, family.user_id, r.from, r.to, kinds).await?;
    let grouped = db::stats::totals_grouped(pool, family.user_id, r.from, r.to, kinds, r.group, r.tz).await?;
    let top = db::stats::top_between(pool, family.user_id, r.from, r.to, kinds, 10).await?;
    Ok(Json(Stats {
        from: r.from,
        to: r.to,
        group: r.group.into(),
        listened_ms: totals.iter().map(|k| k.seconds_listened).sum::<i64>() * 1000,
        plays: totals.iter().map(|k| k.session_count).sum(),
        by_kind: by_kind(&totals),
        buckets: buckets(grouped),
        top: top
            .into_iter()
            .map(|t| TopItem { kind: item_kind(&t.media_kind).into(), id: t.item_id, listened_ms: t.seconds_listened * 1000, plays: t.session_count })
            .collect(),
        streak_days: db::stats::current_streak_days(pool, family.user_id, r.tz).await?,
        finished_books: db::stats::completed_items_count(pool, family.user_id).await?,
    }))
}

// ── History ──────────────────────────────────────────────────────────────────

/// One play: what `POST /me/plays` recorded, or what a progress save stood for.
#[derive(Serialize, ToSchema)]
pub struct PlayRecord {
    pub id: Uuid,
    /// `track`, `book` or `episode`.
    pub kind: String,
    pub item_id: Uuid,
    /// A book's: the file played.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub listened_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    pub device_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_reason: Option<String>,
    /// The device's id for it, as sent to `POST /me/plays`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// `reported` (a device sent it) or `derived` (from a client that only saves its position).
    pub source: String,
}

pub(crate) fn play_record(row: db::stats::PlayRow) -> PlayRecord {
    let (kind, item_id, file_id) = match row.media_kind.as_str() {
        db::access::AUDIOBOOK => ("book", row.item_id, row.part_id),
        // An episode is stored as its show with the episode as the part.
        db::access::PODCAST => ("episode", row.part_id.unwrap_or(row.item_id), None),
        _ => ("track", row.item_id, None),
    };
    PlayRecord {
        id: row.id,
        kind: kind.into(),
        item_id,
        file_id,
        started_at: row.started_at,
        ended_at: row.ended_at,
        listened_ms: i64::from(row.seconds_listened) * 1000,
        speed: row.playback_speed,
        device_kind: row.device_kind,
        ended_reason: row.ended_reason,
        client_id: row.client_session_id,
        source: row.source,
    }
}

/// The caller's plays, newest first.
#[utoipa::path(get, path = "/me/plays", tag = "stats", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<PlayRecord>), (status = 400, description = "`invalid_cursor`, `invalid_limit`", body = crate::v2::problem::ProblemBody)))]
pub async fn my_plays(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<PlayRecord>>, Problem> {
    let limit = page.limit()?;
    let rows = db::stats::plays_page(state.db(), family.user_id, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |r| (r.started_at, r.id));
    Ok(Json(Page { items: page.items.into_iter().map(play_record).collect(), next: page.next, included: None }))
}

// ── The household ────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct MemberStats {
    pub user_id: Uuid,
    pub listened_ms: i64,
    pub plays: i64,
    pub by_kind: Vec<KindTotal>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FamilyStatsQuery {
    /// As on `/me/stats`.
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// Comma-separated `track`, `book`, `episode`.
    pub kind: Option<String>,
}

/// The household's listening over a range, for its admins: the asking admin and every member
/// who shares their statistics (or whom an admin supervises); the others are absent. Names come
/// from `GET /family/members`.
#[utoipa::path(get, path = "/family/stats", tag = "stats", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(FamilyStatsQuery),
    responses((status = 200, body = Vec<MemberStats>),
        (status = 400, description = "`invalid_range`, `unsupported_kind`", body = crate::v2::problem::ProblemBody)))]
pub async fn family_stats(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Query(q): Query<FamilyStatsQuery>) -> Result<Json<Vec<MemberStats>>, Problem> {
    let r = range(&StatsQuery { from: q.from, to: q.to, group: None, kind: q.kind, tz_offset_minutes: None }, Utc::now())?;
    let pool = state.db();
    let mut out = Vec::new();
    for user_id in db::stats::sharing_members(pool, family.family_id, family.user_id).await? {
        let totals = db::stats::totals_between(pool, user_id, r.from, r.to, r.kinds.as_deref()).await?;
        out.push(MemberStats {
            user_id,
            listened_ms: totals.iter().map(|k| k.seconds_listened).sum::<i64>() * 1000,
            plays: totals.iter().map(|k| k.session_count).sum(),
            by_kind: by_kind(&totals),
        });
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(from: Option<&str>, to: Option<&str>, group: Option<&str>, kind: Option<&str>, tz: Option<i32>) -> StatsQuery {
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        StatsQuery { from: from.map(at), to: to.map(at), group: group.map(Into::into), kind: kind.map(Into::into), tz_offset_minutes: tz }
    }

    #[test]
    fn a_range_is_checked() {
        let now: DateTime<Utc> = "2026-11-17T12:00:00Z".parse().unwrap();
        let r = range(&q(None, None, None, None, None), now).unwrap();
        assert_eq!((r.to, r.to - r.from, r.group, r.kinds, r.tz), (now, Duration::days(30), "day", None, 0));
        let r = range(&q(None, None, Some("month"), Some("book, episode,book"), Some(60)), now).unwrap();
        assert_eq!((r.group, r.kinds, r.tz), ("month", Some(vec!["audiobook".to_string(), "podcast".to_string()]), 60));
        let code = |x: StatsQuery| range(&x, now).unwrap_err().code().to_string();
        assert_eq!(code(q(Some("2026-11-17T12:00:00Z"), Some("2026-11-17T12:00:00Z"), None, None, None)), "invalid_range");
        assert_eq!(code(q(Some("2000-01-01T00:00:00Z"), None, None, None, None)), "invalid_range");
        assert_eq!(code(q(None, None, Some("year"), None, None)), "invalid_group");
        assert_eq!(code(q(None, None, None, Some("album"), None)), "unsupported_kind");
        assert_eq!(code(q(None, None, None, None, Some(999))), "invalid_tz");
    }

    #[test]
    fn buckets_gather_their_kinds() {
        let d = |s: &str| s.parse::<chrono::NaiveDate>().unwrap();
        let b = buckets(vec![(d("2026-11-16"), "music".into(), 60, 2), (d("2026-11-16"), "audiobook".into(), 600, 1), (d("2026-11-17"), "podcast".into(), 30, 1)]);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].start.as_str(), b[0].listened_ms, b[0].plays), ("2026-11-16", 660_000, 3));
        assert_eq!(b[0].by_kind[1], KindTotal { kind: "book".into(), listened_ms: 600_000, plays: 1 });
        assert_eq!(b[1].by_kind[0].kind, "episode");
    }

    #[test]
    fn a_play_names_what_was_played() {
        let (feed, episode, book, file) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let row = |media: &str, item, part| db::stats::PlayRow {
            id: Uuid::new_v4(),
            media_kind: media.into(),
            item_id: item,
            part_id: part,
            started_at: Utc::now(),
            ended_at: Utc::now(),
            seconds_listened: 90,
            playback_speed: None,
            device_kind: "ios".into(),
            source: "reported".into(),
            client_session_id: Some("c1".into()),
            ended_reason: None,
        };
        let p = play_record(row("podcast", feed, Some(episode)));
        assert_eq!((p.kind.as_str(), p.item_id, p.file_id, p.listened_ms), ("episode", episode, None, 90_000));
        let b = play_record(row("audiobook", book, Some(file)));
        assert_eq!((b.kind.as_str(), b.item_id, b.file_id), ("book", book, Some(file)));
    }
}
