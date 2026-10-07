// SPDX-License-Identifier: AGPL-3.0-or-later
/// Statistics module — personal listening board and family roll-up.
///
/// Listening history is personal data: a member's stats are private by
/// default and a family admin sees them only with consent, or for a
/// restricted (managed) account. See decision D2 in the family plan.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::FamilyContext;
use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatsQuery {
    /// `7d`, `30d`, `365d`, or `all`. Defaults to `30d`.
    #[serde(default)]
    pub range: Option<String>,
    /// Minutes east of UTC, so day buckets match the listener's local day.
    /// A phone sends its current offset; the web UI sends
    /// `-new Date().getTimezoneOffset()`.
    #[serde(default)]
    pub tz_offset_minutes: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct StatsResponse {
    pub range: String,
    pub total_seconds: i64,
    pub by_kind: Vec<KindTotalResponse>,
    pub by_day: Vec<DayTotalResponse>,
    /// `by_day` split by media kind; days without listening are left out.
    pub by_day_kind: Vec<DayKindTotalResponse>,
    pub top_items: Vec<TopItemResponse>,
    /// Consecutive days ending today with any listening.
    pub streak_days: i64,
    /// Lifetime count of finished audiobooks, regardless of the requested range.
    pub completed_items: i64,
}

#[derive(Serialize, ToSchema)]
pub struct KindTotalResponse {
    pub media_kind: String,
    pub seconds: i64,
    pub sessions: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DayTotalResponse {
    /// `YYYY-MM-DD` in the requested timezone.
    pub day: String,
    pub seconds: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DayKindTotalResponse {
    /// `YYYY-MM-DD` in the requested timezone.
    pub day: String,
    pub media_kind: String,
    pub seconds: i64,
}

#[derive(Serialize, ToSchema)]
pub struct TopItemResponse {
    pub media_kind: String,
    pub item_id: String,
    /// Resolved from the owning table; null if the item was deleted.
    pub title: Option<String>,
    pub seconds: i64,
    pub sessions: i64,
}

#[derive(Serialize, ToSchema)]
pub struct HistoryEntry {
    pub id: String,
    pub media_kind: String,
    pub item_id: String,
    pub part_id: Option<String>,
    pub title: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub seconds: i32,
    pub device_kind: String,
    /// `reported` (client-sent) or `derived` (inferred from progress).
    pub source: String,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HistoryQuery {
    /// 1–200, default 50.
    #[serde(default = "default_history_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_history_limit() -> i64 {
    50
}

#[derive(Serialize, ToSchema)]
pub struct FamilyStatsEntry {
    pub user_id: String,
    pub display_name: String,
    pub display_label: Option<String>,
    /// Absent when the member keeps their statistics private.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_kind: Option<Vec<KindTotalResponse>>,
    /// True when this member has not shared their statistics.
    pub hidden: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetStatsVisibilityRequest {
    /// `private` or `family_admin`.
    pub stats_visibility: String,
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(my_stats))
        .routes(routes!(my_history))
        .routes(routes!(set_my_stats_visibility))
        .routes(routes!(family_stats))
        .routes(routes!(set_member_stats_visibility))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/stats/me?range=30d&tz_offset_minutes=60
///
/// The caller's own listening statistics for a range.
#[utoipa::path(get, path = "/me", tag = "stats", security(("bearer" = [])),
    params(StatsQuery),
    responses((status = 200, body = StatsResponse),
        (status = 400, description = "Unknown range or timezone offset out of bounds", body = crate::http::openapi::ErrorBody)))]
async fn my_stats(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<StatsResponse>, AuthError> {
    let (range, since) = resolve_range(q.range.as_deref())?;
    let tz = normalize_tz(q.tz_offset_minutes)?;

    Ok(Json(
        build_stats(&state, family.user_id, &range, since, tz).await?,
    ))
}

/// GET /api/v1/stats/me/history — the raw session log, newest first.
#[utoipa::path(get, path = "/me/history", tag = "stats", security(("bearer" = [])),
    params(HistoryQuery),
    responses((status = 200, body = Vec<HistoryEntry>)))]
async fn my_history(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryEntry>>, AuthError> {
    let limit = q.limit.clamp(1, 200);
    let offset = q.offset.max(0);

    let rows = db::stats::recent_sessions(state.db(), family.user_id, limit, offset)
        .await
        .map_err(AuthError::Internal)?;

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let title = resolve_title(&state, &row.media_kind, row.item_id).await?;
        entries.push(HistoryEntry {
            id: row.id.to_string(),
            media_kind: row.media_kind,
            item_id: row.item_id.to_string(),
            part_id: row.part_id.map(|p| p.to_string()),
            title,
            started_at: row.started_at.to_rfc3339(),
            ended_at: row.ended_at.to_rfc3339(),
            seconds: row.seconds_listened,
            device_kind: row.device_kind,
            source: row.source,
        });
    }

    Ok(Json(entries))
}

/// PUT /api/v1/stats/me/visibility — a member decides whether their family
/// admin may see their statistics.
#[utoipa::path(put, path = "/me/visibility", tag = "stats", security(("bearer" = [])),
    request_body = SetStatsVisibilityRequest,
    responses((status = 204, description = "Saved"),
        (status = 400, description = "Unknown visibility", body = crate::http::openapi::ErrorBody)))]
async fn set_my_stats_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<SetStatsVisibilityRequest>,
) -> Result<StatusCode, AuthError> {
    let visibility = parse_stats_visibility(&body.stats_visibility)?;

    db::stats::set_stats_visibility(state.db(), family.family_id, family.user_id, visibility)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/v1/stats/family/members/{user_id}/visibility
///
/// A family admin may only flip this for a **restricted** member — one who
/// already carries a `deny_all` policy, i.e. an account they manage. Adults
/// with unrestricted access control their own visibility, so a parent cannot
/// silently start watching another adult.
#[utoipa::path(put, path = "/family/members/{user_id}/visibility", tag = "stats", security(("bearer" = [])),
    params(("user_id" = Uuid, Path, description = "The family member")),
    request_body = SetStatsVisibilityRequest,
    responses((status = 204, description = "Saved"),
        (status = 400, description = "Unknown visibility", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Not a member of the caller's family", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "Not a family admin, or the member is not restricted", body = crate::http::openapi::ErrorBody)))]
async fn set_member_stats_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<SetStatsVisibilityRequest>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    let visibility = parse_stats_visibility(&body.stats_visibility)?;

    if user_id != family.user_id
        && !db::stats::is_restricted_member(state.db(), family.family_id, user_id)
            .await
            .map_err(AuthError::Internal)?
    {
        return Err(AuthError::Forbidden);
    }

    let updated =
        db::stats::set_stats_visibility(state.db(), family.family_id, user_id, visibility)
            .await
            .map_err(AuthError::Internal)?;

    if !updated {
        return Err(AuthError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/stats/family — per-member roll-up for family admins.
/// Members who keep their stats private appear with `hidden: true` and no
/// figures, so the UI can show the roster without leaking totals.
#[utoipa::path(get, path = "/family", tag = "stats", security(("bearer" = [])),
    params(StatsQuery),
    responses((status = 200, body = Vec<FamilyStatsEntry>),
        (status = 400, description = "Unknown range or timezone offset out of bounds", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "Not a family admin", body = crate::http::openapi::ErrorBody)))]
async fn family_stats(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<Vec<FamilyStatsEntry>>, AuthError> {
    family.require_family_admin()?;
    let (_, since) = resolve_range(q.range.as_deref())?;

    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut out = Vec::with_capacity(members.len());
    for member in members {
        let visible = member.user_id == family.user_id
            || db::stats::stats_visibility(state.db(), family.family_id, member.user_id)
                .await
                .map_err(AuthError::Internal)?
                .as_deref()
                == Some("family_admin");

        if !visible {
            out.push(FamilyStatsEntry {
                user_id: member.user_id.to_string(),
                display_name: member.display_name,
                display_label: member.display_label,
                total_seconds: None,
                by_kind: None,
                hidden: true,
            });
            continue;
        }

        let by_kind = db::stats::totals_by_kind(state.db(), member.user_id, since)
            .await
            .map_err(AuthError::Internal)?;

        out.push(FamilyStatsEntry {
            user_id: member.user_id.to_string(),
            display_name: member.display_name,
            display_label: member.display_label,
            total_seconds: Some(by_kind.iter().map(|k| k.seconds_listened).sum()),
            by_kind: Some(by_kind.into_iter().map(kind_to_response).collect()),
            hidden: false,
        });
    }

    Ok(Json(out))
}

// ── Helpers ───────────────────────────────────────────────────────────────

async fn build_stats(
    state: &AppState,
    user_id: Uuid,
    range: &str,
    since: DateTime<Utc>,
    tz: i32,
) -> Result<StatsResponse, AuthError> {
    let pool = state.db();

    let by_kind = db::stats::totals_by_kind(pool, user_id, since)
        .await
        .map_err(AuthError::Internal)?;
    let by_day = db::stats::totals_by_day(pool, user_id, since, tz)
        .await
        .map_err(AuthError::Internal)?;
    let by_day_kind = db::stats::totals_by_day_and_kind(pool, user_id, since, tz)
        .await
        .map_err(AuthError::Internal)?;
    let tops = db::stats::top_items(pool, user_id, since, 10)
        .await
        .map_err(AuthError::Internal)?;
    let streak = db::stats::current_streak_days(pool, user_id, tz)
        .await
        .map_err(AuthError::Internal)?;
    let completed_items = db::stats::completed_items_count(pool, user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut top_items = Vec::with_capacity(tops.len());
    for item in tops {
        let title = resolve_title(state, &item.media_kind, item.item_id).await?;
        top_items.push(TopItemResponse {
            media_kind: item.media_kind,
            item_id: item.item_id.to_string(),
            title,
            seconds: item.seconds_listened,
            sessions: item.session_count,
        });
    }

    Ok(StatsResponse {
        range: range.to_string(),
        total_seconds: by_kind.iter().map(|k| k.seconds_listened).sum(),
        by_kind: by_kind.into_iter().map(kind_to_response).collect(),
        by_day: by_day
            .into_iter()
            .map(|d| DayTotalResponse {
                day: d.day.to_string(),
                seconds: d.seconds_listened,
            })
            .collect(),
        by_day_kind: by_day_kind
            .into_iter()
            .map(|d| DayKindTotalResponse {
                day: d.day.to_string(),
                media_kind: d.media_kind,
                seconds: d.seconds_listened,
            })
            .collect(),
        top_items,
        streak_days: streak,
        completed_items,
    })
}

fn kind_to_response(k: db::stats::KindTotal) -> KindTotalResponse {
    KindTotalResponse {
        media_kind: k.media_kind,
        seconds: k.seconds_listened,
        sessions: k.session_count,
    }
}

/// Look up a display title for a session's item. Deleted items resolve to
/// `None` rather than failing the whole request — history outlives content.
async fn resolve_title(
    state: &AppState,
    media_kind: &str,
    item_id: Uuid,
) -> Result<Option<String>, AuthError> {
    let sql = match media_kind {
        db::access::AUDIOBOOK => "SELECT title FROM audiobook_books WHERE id = $1",
        db::access::PODCAST => "SELECT title FROM podcast_feeds WHERE id = $1",
        db::access::MUSIC => "SELECT title FROM music_tracks WHERE id = $1",
        _ => return Ok(None),
    };

    sqlx::query_scalar::<_, String>(sql)
        .bind(item_id)
        .fetch_optional(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))
}

fn resolve_range(range: Option<&str>) -> Result<(String, DateTime<Utc>), AuthError> {
    let range = range.unwrap_or("30d").trim();
    let since = match range {
        "7d" => Utc::now() - Duration::days(7),
        "30d" => Utc::now() - Duration::days(30),
        "90d" => Utc::now() - Duration::days(90),
        "365d" => Utc::now() - Duration::days(365),
        "all" => DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now),
        _ => {
            return Err(AuthError::BadRequest(
                "range must be '7d', '30d', '90d', '365d', or 'all'".to_string(),
            ));
        }
    };
    Ok((range.to_string(), since))
}

fn normalize_tz(offset: Option<i32>) -> Result<i32, AuthError> {
    let offset = offset.unwrap_or(0);
    // UTC-12:00 .. UTC+14:00 covers every real zone.
    if !(-720..=840).contains(&offset) {
        return Err(AuthError::BadRequest(
            "tz_offset_minutes must be between -720 and 840".to_string(),
        ));
    }
    Ok(offset)
}

fn parse_stats_visibility(value: &str) -> Result<&'static str, AuthError> {
    match value.trim() {
        "private" => Ok("private"),
        "family_admin" => Ok("family_admin"),
        _ => Err(AuthError::BadRequest(
            "stats_visibility must be 'private' or 'family_admin'".to_string(),
        )),
    }
}
