// SPDX-License-Identifier: AGPL-3.0-or-later
/// Listening history and statistics (migration 0020).
///
/// Sessions arrive one of two ways:
///   * **reported** — a client posts explicit spans to `/playback/sessions`,
///     batched and idempotent so Android WorkManager / iOS background tasks
///     can retry safely.
///   * **derived** — inferred from a progress update, so clients that only
///     ever save a position (today's web UI, Subsonic apps) still produce
///     stats. Derivation is skipped for users whose clients report
///     explicitly, which is what keeps the two from double-counting.
use anyhow::Context;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Longest plausible single listening span. Anything beyond this is a client
/// bug or a device that slept mid-playback; clamped rather than trusted.
const MAX_SESSION_SECS: i64 = 6 * 3600;

/// A client-reported listening span.
#[derive(Debug, Clone)]
pub struct SessionInput {
    pub media_kind: String,
    pub item_id: Uuid,
    pub part_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub seconds_listened: i32,
    pub playback_speed: Option<f64>,
    pub device_kind: String,
    pub client_session_id: Option<String>,
    /// Why playback stopped, when the client said. None means "not reported",
    /// which is not the same as "stopped normally" — it contributes no signal.
    pub ended_reason: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub media_kind: String,
    pub item_id: Uuid,
    pub part_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub seconds_listened: i32,
    pub device_kind: String,
    pub source: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KindTotal {
    pub media_kind: String,
    pub seconds_listened: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DayTotal {
    pub day: NaiveDate,
    pub seconds_listened: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DayKindTotal {
    pub day: NaiveDate,
    pub media_kind: String,
    pub seconds_listened: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ItemTotal {
    pub media_kind: String,
    pub item_id: Uuid,
    pub seconds_listened: i64,
    pub session_count: i64,
}

/// Insert a batch of reported sessions, ignoring any whose
/// `client_session_id` was already stored. Returns how many were new.
///
/// Callers should treat a duplicate as success: a retry after a lost response
/// must be a no-op, not an error.
pub async fn record_sessions(
    pool: &PgPool,
    user_id: Uuid,
    sessions: &[SessionInput],
) -> anyhow::Result<u64> {
    if sessions.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await.context("db: begin session batch")?;
    let mut inserted = 0_u64;

    for s in sessions {
        let seconds = s.seconds_listened.clamp(0, MAX_SESSION_SECS as i32);

        let result = sqlx::query(
            "INSERT INTO listening_sessions
                 (user_id, media_kind, item_id, part_id, started_at, ended_at,
                  seconds_listened, playback_speed, device_kind, source, client_session_id,
                  ended_reason)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'reported', $10, $11)
             ON CONFLICT (user_id, client_session_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(&s.media_kind)
        .bind(s.item_id)
        .bind(s.part_id)
        .bind(s.started_at)
        .bind(s.ended_at)
        .bind(seconds)
        .bind(s.playback_speed)
        .bind(&s.device_kind)
        .bind(s.client_session_id.as_deref())
        .bind(s.ended_reason.as_deref())
        .execute(&mut *tx)
        .await
        .context("db: insert listening session")?;

        inserted += result.rows_affected();
    }

    tx.commit().await.context("db: commit session batch")?;
    Ok(inserted)
}

/// True when this user's clients report sessions explicitly (any reported row
/// in the last 7 days). Such users get no derived rows, so the two sources
/// never double-count the same playback.
pub async fn uses_explicit_sessions(pool: &PgPool, user_id: Uuid) -> anyhow::Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM listening_sessions
             WHERE user_id = $1 AND source = 'reported'
               AND started_at > now() - INTERVAL '7 days')",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: check explicit session usage")
}

/// Record a listening span inferred from a progress jump.
///
/// `delta_secs` is how far the position advanced. Non-positive deltas (seeks
/// backwards, no-op saves) and implausibly large ones (a seek to the end)
/// are ignored rather than clamped: a forward seek is not listening.
pub async fn derive_from_progress(
    pool: &PgPool,
    user_id: Uuid,
    media_kind: &str,
    item_id: Uuid,
    part_id: Option<Uuid>,
    delta_secs: f64,
    device_kind: &str,
) -> anyhow::Result<()> {
    // `!is_finite()` keeps the NaN rejection the negated comparison used to do.
    if !delta_secs.is_finite() || delta_secs <= 0.0 || delta_secs > MAX_SESSION_SECS as f64 {
        return Ok(());
    }

    if uses_explicit_sessions(pool, user_id).await? {
        return Ok(());
    }

    let ended_at = Utc::now();
    let started_at = ended_at - Duration::seconds(delta_secs as i64);

    sqlx::query(
        "INSERT INTO listening_sessions
             (user_id, media_kind, item_id, part_id, started_at, ended_at,
              seconds_listened, device_kind, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'derived')",
    )
    .bind(user_id)
    .bind(media_kind)
    .bind(item_id)
    .bind(part_id)
    .bind(started_at)
    .bind(ended_at)
    .bind(delta_secs as i32)
    .bind(device_kind)
    .execute(pool)
    .await
    .context("db: insert derived listening session")?;

    Ok(())
}

// ── Aggregates ────────────────────────────────────────────────────────────
//
// `tz_offset_minutes` shifts UTC timestamps into the caller's local time
// before bucketing by day, so "yesterday" means the listener's yesterday
// rather than the server's.

pub async fn totals_by_kind(
    pool: &PgPool,
    user_id: Uuid,
    since: DateTime<Utc>,
) -> anyhow::Result<Vec<KindTotal>> {
    sqlx::query_as::<_, KindTotal>(
        "SELECT media_kind,
                COALESCE(SUM(seconds_listened), 0)::bigint AS seconds_listened,
                COUNT(*)::bigint AS session_count
         FROM listening_sessions
         WHERE user_id = $1 AND started_at >= $2
         GROUP BY media_kind
         ORDER BY media_kind",
    )
    .bind(user_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .context("db: listening totals by kind")
}

pub async fn totals_by_day(
    pool: &PgPool,
    user_id: Uuid,
    since: DateTime<Utc>,
    tz_offset_minutes: i32,
) -> anyhow::Result<Vec<DayTotal>> {
    sqlx::query_as::<_, DayTotal>(
        "SELECT (started_at + make_interval(mins => $3))::date AS day,
                COALESCE(SUM(seconds_listened), 0)::bigint AS seconds_listened
         FROM listening_sessions
         WHERE user_id = $1 AND started_at >= $2
         GROUP BY 1
         ORDER BY 1",
    )
    .bind(user_id)
    .bind(since)
    .bind(tz_offset_minutes)
    .fetch_all(pool)
    .await
    .context("db: listening totals by day")
}

/// Like `totals_by_day`, split by media kind — the per-cloud activity grids.
pub async fn totals_by_day_and_kind(
    pool: &PgPool,
    user_id: Uuid,
    since: DateTime<Utc>,
    tz_offset_minutes: i32,
) -> anyhow::Result<Vec<DayKindTotal>> {
    sqlx::query_as::<_, DayKindTotal>(
        "SELECT (started_at + make_interval(mins => $3))::date AS day,
                media_kind,
                COALESCE(SUM(seconds_listened), 0)::bigint AS seconds_listened
         FROM listening_sessions
         WHERE user_id = $1 AND started_at >= $2
         GROUP BY 1, 2
         ORDER BY 1, 2",
    )
    .bind(user_id)
    .bind(since)
    .bind(tz_offset_minutes)
    .fetch_all(pool)
    .await
    .context("db: listening totals by day and kind")
}

pub async fn top_items(
    pool: &PgPool,
    user_id: Uuid,
    since: DateTime<Utc>,
    limit: i64,
) -> anyhow::Result<Vec<ItemTotal>> {
    sqlx::query_as::<_, ItemTotal>(
        "SELECT media_kind, item_id,
                COALESCE(SUM(seconds_listened), 0)::bigint AS seconds_listened,
                COUNT(*)::bigint AS session_count
         FROM listening_sessions
         WHERE user_id = $1 AND started_at >= $2
         GROUP BY media_kind, item_id
         ORDER BY seconds_listened DESC
         LIMIT $3",
    )
    .bind(user_id)
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: top listened items")
}

pub async fn recent_sessions(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> anyhow::Result<Vec<SessionRow>> {
    sqlx::query_as::<_, SessionRow>(
        "SELECT id, media_kind, item_id, part_id, started_at, ended_at,
                seconds_listened, device_kind, source
         FROM listening_sessions
         WHERE user_id = $1
         ORDER BY started_at DESC
         LIMIT $2 OFFSET $3",
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .context("db: recent listening sessions")
}

/// Consecutive days ending today (in the caller's timezone) with any
/// listening. Returns 0 when there was none today.
pub async fn current_streak_days(
    pool: &PgPool,
    user_id: Uuid,
    tz_offset_minutes: i32,
) -> anyhow::Result<i64> {
    let days: Vec<NaiveDate> = sqlx::query_scalar(
        "SELECT DISTINCT (started_at + make_interval(mins => $2))::date AS day
         FROM listening_sessions
         WHERE user_id = $1
           AND started_at > now() - INTERVAL '400 days'
         ORDER BY day DESC",
    )
    .bind(user_id)
    .bind(tz_offset_minutes)
    .fetch_all(pool)
    .await
    .context("db: listening streak")?;

    let today = (Utc::now() + Duration::minutes(tz_offset_minutes as i64)).date_naive();
    let mut streak = 0_i64;
    let mut expected = today;

    for day in days {
        if day == expected {
            streak += 1;
            expected -= Duration::days(1);
        } else if day < expected {
            break;
        }
    }

    Ok(streak)
}

/// Count of audiobooks this user has finished. `audiobook_progress` is one row per
/// (user, book) — the whole-book playback head, not per file — so `completed` here is
/// already a whole-book flag, set true when the last file of a book finishes playing
/// (see `PlaybackEngine.handleFileDidFinish` on the clients). No `since`/range filter:
/// this is a lifetime count, unlike the rest of this module.
pub async fn completed_items_count(pool: &PgPool, user_id: Uuid) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM audiobook_progress WHERE user_id = $1 AND completed = true",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: completed audiobook count")
}

// ── Daily rollup ──────────────────────────────────────────────────────────

/// Rebuild the rollup for the last `days` days. Idempotent, so the nightly
/// job can safely re-run and pick up late-arriving offline batches.
pub async fn rebuild_daily_rollup(pool: &PgPool, days: i32) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "INSERT INTO listening_daily (user_id, day, media_kind, seconds_listened, session_count)
         SELECT user_id,
                started_at::date,
                media_kind,
                SUM(seconds_listened)::bigint,
                COUNT(*)::int
         FROM listening_sessions
         WHERE started_at >= (now() - make_interval(days => $1))
         GROUP BY user_id, started_at::date, media_kind
         ON CONFLICT (user_id, day, media_kind) DO UPDATE
             SET seconds_listened = EXCLUDED.seconds_listened,
                 session_count    = EXCLUDED.session_count,
                 updated_at       = now()",
    )
    .bind(days)
    .execute(pool)
    .await
    .context("db: rebuild listening rollup")?;

    Ok(result.rows_affected())
}

// ── Stats privacy (D2) ────────────────────────────────────────────────────

/// Whether a family admin may read this member's statistics.
pub async fn stats_visibility(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT stats_visibility FROM family_members
         WHERE family_id = $1 AND user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: read stats visibility")
}

pub async fn set_stats_visibility(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    visibility: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE family_members SET stats_visibility = $3
         WHERE family_id = $1 AND user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(visibility)
    .execute(pool)
    .await
    .context("db: set stats visibility")?;

    Ok(result.rows_affected() == 1)
}

/// True when the member carries any `deny_all` policy — a restricted
/// ("managed") account, typically a child's. Family admins may enable stats
/// visibility for these without the member's own consent; see D2.
pub async fn is_restricted_member(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM member_media_policy
             WHERE family_id = $1 AND user_id = $2 AND policy = 'deny_all')",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: check restricted member")
}

/// Streaming activity by rolling window (last 1/7/30 days), instance-wide —
/// for the admin dashboard. Reads `listening_sessions` directly rather than
/// the `listening_daily` rollup: nothing currently enqueues the job that
/// would populate that table, and at this deployment's actual scale a plain
/// scan of the last 30 days is cheap. `(seconds, sessions, active_listeners)`
/// per window.
pub async fn streaming_totals(
    pool: &PgPool,
) -> anyhow::Result<((i64, i64, i64), (i64, i64, i64), (i64, i64, i64))> {
    sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        "SELECT
             COALESCE(SUM(seconds_listened) FILTER (WHERE started_at >= now() - interval '1 day'), 0)::BIGINT,
             COUNT(*) FILTER (WHERE started_at >= now() - interval '1 day')::BIGINT,
             COUNT(DISTINCT user_id) FILTER (WHERE started_at >= now() - interval '1 day')::BIGINT,
             COALESCE(SUM(seconds_listened) FILTER (WHERE started_at >= now() - interval '7 days'), 0)::BIGINT,
             COUNT(*) FILTER (WHERE started_at >= now() - interval '7 days')::BIGINT,
             COUNT(DISTINCT user_id) FILTER (WHERE started_at >= now() - interval '7 days')::BIGINT,
             COALESCE(SUM(seconds_listened) FILTER (WHERE started_at >= now() - interval '30 days'), 0)::BIGINT,
             COUNT(*) FILTER (WHERE started_at >= now() - interval '30 days')::BIGINT,
             COUNT(DISTINCT user_id) FILTER (WHERE started_at >= now() - interval '30 days')::BIGINT
         FROM listening_sessions
         WHERE started_at >= now() - interval '30 days'",
    )
    .fetch_one(pool)
    .await
    .map(|(s1, n1, l1, s7, n7, l7, s30, n30, l30)| ((s1, n1, l1), (s7, n7, l7), (s30, n30, l30)))
    .context("db: streaming totals")
}
