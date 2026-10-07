// SPDX-License-Identifier: AGPL-3.0-or-later
/// Per-member access control over shared family content.
///
/// The visibility rule itself lives in the `audio2_can_access` SQL function
/// (migration 0019) so the REST API, the Subsonic surface, and every list
/// query evaluate identical logic. [`VISIBLE`] is the predicate to paste into
/// list queries; [`can_listen`] is the point check used before streaming.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Media kinds that can be shared and granted.
pub const AUDIOBOOK: &str = "audiobook";
pub const PODCAST: &str = "podcast";
pub const MUSIC: &str = "music";

pub fn is_valid_kind(kind: &str) -> bool {
    matches!(kind, AUDIOBOOK | PODCAST | MUSIC)
}

/// Which table a `media_kind` addresses. Grants store a bare `item_id`, so
/// this is how we resolve it back to a row.
///
/// Only these three tables carry item-level grants. Playlists, collections,
/// and series are shareable but not individually grantable: they evaluate
/// against the member's default policy for their parent kind (`music` for
/// playlists, `audiobook` for collections and series), which is why a member
/// denied all music also stops seeing shared music playlists.
pub fn table_for_kind(kind: &str) -> Option<&'static str> {
    match kind {
        AUDIOBOOK => Some("audiobook_books"),
        PODCAST => Some("podcast_feeds"),
        MUSIC => Some("music_tracks"),
        _ => None,
    }
}

/// Visibility predicate for list queries. Interpolate it into a `WHERE`
/// clause over a table aliased `t`, then bind, in order:
///   $1 viewer user_id, $2 viewer family_id, $3 is_family_admin, $4 media_kind
///
/// ```ignore
/// let sql = format!(
///     "SELECT t.* FROM audiobook_books t WHERE {VISIBLE} ORDER BY t.created_at DESC"
/// );
/// sqlx::query_as(&sql).bind(user_id).bind(family_id).bind(is_admin).bind(AUDIOBOOK)
/// ```
///
/// It is `audio2_can_access` (migration 0019) written out inline: the owner
/// sees their own items; a family's shared items are visible to its admins,
/// and to members unless a per-item grant or their media policy denies it.
/// Inline, PostgreSQL evaluates the policy once per query and the grants as
/// one hashed set, instead of calling the function for every row — at
/// 600,000 tracks that is 0.2 s instead of 1.6 s for a family's album list
/// (docs/CAPACITY.md). Keep the two in step.
pub const VISIBLE: &str = "(t.user_id = $1
    OR (t.family_id IS NOT NULL AND t.family_id = $2 AND (
        $3
        OR t.id IN (SELECT g.item_id FROM content_grants g
                     WHERE g.user_id = $1 AND g.media_kind = $4 AND g.effect = 'allow')
        OR (t.id NOT IN (SELECT g.item_id FROM content_grants g
                          WHERE g.user_id = $1 AND g.media_kind = $4 AND g.effect = 'deny')
            AND COALESCE((SELECT p.policy FROM member_media_policy p
                           WHERE p.family_id = $2 AND p.user_id = $1 AND p.media_kind = $4),
                          'allow_all') = 'allow_all'))))";

/// [`VISIBLE`] for a fixed media kind (an SQL literal such as `'music'`, or a
/// column such as `t.media_kind`) in place of the `$4` parameter.
pub fn visible_for(kind_sql: &str) -> String {
    VISIBLE.replace("$4", kind_sql)
}

/// Everything needed to evaluate visibility for one request.
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub user_id: Uuid,
    pub family_id: Uuid,
    pub is_family_admin: bool,
}

// ── Visibility (which "folder" an item lives in) ──────────────────────────

pub const VIS_PRIVATE: &str = "private";
pub const VIS_FAMILY: &str = "family";

/// Translate an API `visibility` value into the `family_id` column value.
///
/// An absent or empty value resolves to **private**: uploads must never
/// expose content because a client forgot a field, and it keeps existing
/// clients (which send nothing) behaving exactly as they did before.
pub fn family_id_for_visibility(
    value: Option<&str>,
    family_id: Uuid,
) -> Result<Option<Uuid>, &'static str> {
    match value.map(str::trim) {
        None | Some("") | Some(VIS_PRIVATE) => Ok(None),
        Some(VIS_FAMILY) => Ok(Some(family_id)),
        Some(_) => Err("visibility must be 'private' or 'family'"),
    }
}

/// The API-facing visibility of a stored row.
pub fn visibility_of(family_id: Option<Uuid>) -> &'static str {
    if family_id.is_some() { VIS_FAMILY } else { VIS_PRIVATE }
}

/// Can this viewer play this item? Used before handing out a presigned URL.
/// Returns false for items that do not exist, so callers can treat false as
/// "404 or 403" without leaking which.
pub async fn can_listen(
    pool: &PgPool,
    viewer: Viewer,
    kind: &str,
    item_id: Uuid,
) -> anyhow::Result<bool> {
    let Some(table) = table_for_kind(kind) else {
        return Ok(false);
    };

    let sql = format!(
        "SELECT audio2_can_access($1, $2, $3, $4, t.id, t.user_id, t.family_id)
         FROM {table} t WHERE t.id = $5"
    );

    let allowed: Option<bool> = sqlx::query_scalar(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(kind)
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .context("db: check listen access")?;

    Ok(allowed.unwrap_or(false))
}

// ── Sharing ───────────────────────────────────────────────────────────────

/// Share an item with (or unshare it from) the owner's family. Only the owner
/// may do this, so ownership is part of the WHERE clause rather than a
/// separate check. Returns false when the item does not exist or is not owned
/// by `owner_id`.
///
/// Un-sharing also drops the item's grants: they describe access to shared
/// content and would otherwise linger and silently reapply on re-share.
pub async fn set_shared(
    pool: &PgPool,
    owner_id: Uuid,
    family_id: Uuid,
    kind: &str,
    item_id: Uuid,
    shared: bool,
) -> anyhow::Result<bool> {
    let Some(table) = table_for_kind(kind) else {
        return Ok(false);
    };

    let mut tx = pool.begin().await.context("db: begin share update")?;

    // updated_at moves with the change: /library/changes is keyed on it, so a share or
    // unshare that left it alone would never reach any client — their caches would keep
    // showing the old sharing state until something unrelated touched the row.
    let sql = format!(
        "UPDATE {table} SET family_id = $1, updated_at = now() WHERE id = $2 AND user_id = $3"
    );
    let result = sqlx::query(&sql)
        .bind(shared.then_some(family_id))
        .bind(item_id)
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .context("db: update item sharing")?;

    if result.rows_affected() != 1 {
        tx.rollback().await.ok();
        return Ok(false);
    }

    if !shared {
        sqlx::query("DELETE FROM content_grants WHERE media_kind = $1 AND item_id = $2")
            .bind(kind)
            .bind(item_id)
            .execute(&mut *tx)
            .await
            .context("db: clear grants for unshared item")?;
    }

    tx.commit().await.context("db: commit share update")?;
    Ok(true)
}

/// Revert every item a user shared with a family back to private, and drop
/// the grants that referenced them. Called when a member leaves or is removed:
/// content follows its owner out of the family.
pub async fn unshare_all_for_user(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Uuid,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin unshare-all")?;

    for table in [
        "audiobook_books",
        "podcast_feeds",
        "music_tracks",
        "music_playlists",
        "audiobook_collections",
        "audiobook_series",
    ] {
        // Same reason as set_shared: the sync feed is keyed on updated_at. audiobook_series is
        // the one table here without the column — it is not part of /library/changes either.
        let touch = if table == "audiobook_series" {
            ""
        } else {
            ", updated_at = now()"
        };
        let sql = format!(
            "UPDATE {table} SET family_id = NULL{touch} WHERE user_id = $1 AND family_id = $2"
        );
        sqlx::query(&sql)
            .bind(user_id)
            .bind(family_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("db: unshare {table}"))?;
    }

    // Grants held BY the departing user in this family are meaningless now…
    sqlx::query("DELETE FROM content_grants WHERE user_id = $1 AND family_id = $2")
        .bind(user_id)
        .bind(family_id)
        .execute(&mut *tx)
        .await
        .context("db: clear departing member grants")?;

    // …as are the per-kind policies that were set for them.
    sqlx::query("DELETE FROM member_media_policy WHERE user_id = $1 AND family_id = $2")
        .bind(user_id)
        .bind(family_id)
        .execute(&mut *tx)
        .await
        .context("db: clear departing member policies")?;

    tx.commit().await.context("db: commit unshare-all")
}

// ── Policies ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberPolicy {
    pub media_kind: String,
    pub policy: String,
}

pub async fn list_policies(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Vec<MemberPolicy>> {
    sqlx::query_as::<_, MemberPolicy>(
        "SELECT media_kind, policy FROM member_media_policy
         WHERE family_id = $1 AND user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list member policies")
}

pub async fn set_policy(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    kind: &str,
    policy: &str,
    updated_by: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO member_media_policy (family_id, user_id, media_kind, policy, updated_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (family_id, user_id, media_kind) DO UPDATE
             SET policy = EXCLUDED.policy,
                 updated_by = EXCLUDED.updated_by,
                 updated_at = now()",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(kind)
    .bind(policy)
    .bind(updated_by)
    .execute(pool)
    .await
    .context("db: set member policy")?;

    Ok(())
}

// ── Grants ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Grant {
    pub media_kind: String,
    pub item_id: Uuid,
    pub effect: String,
}

pub async fn list_grants(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    kind: Option<&str>,
) -> anyhow::Result<Vec<Grant>> {
    sqlx::query_as::<_, Grant>(
        "SELECT media_kind, item_id, effect FROM content_grants
         WHERE family_id = $1 AND user_id = $2
           AND ($3::text IS NULL OR media_kind = $3)
         ORDER BY media_kind, created_at",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(kind)
    .fetch_all(pool)
    .await
    .context("db: list content grants")
}

/// Replace the caller's whole grant set for one media kind. Bulk-replace beats
/// item-by-item edits because the admin UI presents a checkbox list.
///
/// Items are validated to be shared with this family, so a grant can never
/// point at someone's private content.
pub async fn replace_grants(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    kind: &str,
    allow: &[Uuid],
    deny: &[Uuid],
    granted_by: Uuid,
) -> anyhow::Result<()> {
    let Some(table) = table_for_kind(kind) else {
        anyhow::bail!("unknown media kind: {kind}");
    };

    let mut tx = pool.begin().await.context("db: begin grant replace")?;

    sqlx::query(
        "DELETE FROM content_grants
         WHERE family_id = $1 AND user_id = $2 AND media_kind = $3",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(kind)
    .execute(&mut *tx)
    .await
    .context("db: clear previous grants")?;

    let sql = format!(
        "INSERT INTO content_grants (family_id, user_id, media_kind, item_id, effect, granted_by)
         SELECT $1, $2, $3, t.id, $5, $6
         FROM {table} t
         WHERE t.id = ANY($4) AND t.family_id = $1
         ON CONFLICT (user_id, media_kind, item_id) DO UPDATE
             SET effect = EXCLUDED.effect, granted_by = EXCLUDED.granted_by"
    );

    for (items, effect) in [(allow, "allow"), (deny, "deny")] {
        if items.is_empty() {
            continue;
        }
        sqlx::query(&sql)
            .bind(family_id)
            .bind(user_id)
            .bind(kind)
            .bind(items)
            .bind(effect)
            .bind(granted_by)
            .execute(&mut *tx)
            .await
            .context("db: insert grants")?;
    }

    tx.commit().await.context("db: commit grant replace")
}

/// Which members of the family can currently play this item — the reverse
/// view, for a "who can listen" widget on a detail page.
/// Set, for one item, exactly who may hear it — the other direction from [`replace_grants`].
///
/// Stores the **minimum** that produces the asked-for answer rather than a row per member. Each
/// member already has a default policy for this media kind, and `audio2_can_access` reads a grant
/// only as an exception to it, so a member whose policy already gives the wanted answer needs no
/// row at all — and writing one anyway would freeze today's policy into every item, so a later
/// change of that member's default would silently do nothing.
///
/// Owners and family admins are skipped: the function grants them access unconditionally, and a
/// row claiming otherwise would be a stored lie that no screen could honour.
pub async fn set_item_audience(
    pool: &PgPool,
    family_id: Uuid,
    kind: &str,
    item_id: Uuid,
    can_listen: &[(Uuid, bool)],
    granted_by: Uuid,
) -> anyhow::Result<()> {
    let Some(table) = table_for_kind(kind) else {
        anyhow::bail!("unknown media kind: {kind}");
    };
    set_item_audience_in(pool, family_id, kind, table, item_id, can_listen, granted_by).await
}

/// `set_item_audience` for an item that is not in its kind's own table — a music playlist,
/// which `audio2_can_access` evaluates as `music` but which lives in `music_playlists`.
pub async fn set_item_audience_in(
    pool: &PgPool,
    family_id: Uuid,
    kind: &str,
    table: &str,
    item_id: Uuid,
    can_listen: &[(Uuid, bool)],
    granted_by: Uuid,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin audience write")?;

    // The item must belong to this family, and be shared with it. Selecting through the table
    // rather than trusting the path is what stops one family writing grants over another's.
    let owner = sqlx::query_scalar::<_, Uuid>(&format!(
        "SELECT t.user_id FROM {table} t WHERE t.id = $1 AND t.family_id = $2"
    ))
    .bind(item_id)
    .bind(family_id)
    .fetch_optional(&mut *tx)
    .await
    .context("db: check item is shared with this family")?;

    let Some(owner) = owner else {
        anyhow::bail!("item is not shared with this family");
    };

    for (user_id, wanted) in can_listen {
        // The owner's access is unconditional in `audio2_can_access`, so a row about them would
        // never be read. Skipped rather than written, because a grant that cannot take effect is
        // worse than none: it reads back as a decision somebody made.
        if *user_id == owner {
            continue;
        }

        let policy = sqlx::query_scalar::<_, String>(
            "SELECT policy FROM member_media_policy
             WHERE family_id = $1 AND user_id = $2 AND media_kind = $3",
        )
        .bind(family_id)
        .bind(user_id)
        .bind(kind)
        .fetch_optional(&mut *tx)
        .await
        .context("db: read member policy")?
        .unwrap_or_else(|| "allow_all".to_string());

        let default_allows = policy == "allow_all";

        if default_allows == *wanted {
            // The member's own default already says this; drop any leftover exception.
            sqlx::query(
                "DELETE FROM content_grants
                 WHERE family_id = $1 AND user_id = $2 AND media_kind = $3 AND item_id = $4",
            )
            .bind(family_id)
            .bind(user_id)
            .bind(kind)
            .bind(item_id)
            .execute(&mut *tx)
            .await
            .context("db: clear redundant grant")?;
        } else {
            let effect = if *wanted { "allow" } else { "deny" };
            sqlx::query(
                "INSERT INTO content_grants
                     (family_id, user_id, media_kind, item_id, effect, granted_by)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (user_id, media_kind, item_id) DO UPDATE
                     SET effect = EXCLUDED.effect, granted_by = EXCLUDED.granted_by",
            )
            .bind(family_id)
            .bind(user_id)
            .bind(kind)
            .bind(item_id)
            .bind(effect)
            .bind(granted_by)
            .execute(&mut *tx)
            .await
            .context("db: write item grant")?;
        }
    }

    tx.commit().await.context("db: commit audience write")?;
    Ok(())
}

pub async fn item_audience(
    pool: &PgPool,
    family_id: Uuid,
    kind: &str,
    item_id: Uuid,
) -> anyhow::Result<Vec<(Uuid, bool, bool)>> {
    let Some(table) = table_for_kind(kind) else {
        return Ok(Vec::new());
    };
    item_audience_in(pool, family_id, kind, table, item_id).await
}

/// `item_audience` for an item outside its kind's own table (a music playlist).
pub async fn item_audience_in(
    pool: &PgPool,
    family_id: Uuid,
    kind: &str,
    table: &str,
    item_id: Uuid,
) -> anyhow::Result<Vec<(Uuid, bool, bool)>> {

    // The third column is why a row cannot be toggled, not a second opinion on access: owners and
    // family admins are allowed by `audio2_can_access` before any grant is consulted, so a screen
    // offering a switch for them would be offering one that does nothing.
    let sql = format!(
        "SELECT fm.user_id,
                audio2_can_access(fm.user_id, $1, fm.role = 'family_admin',
                                  $2, t.id, t.user_id, t.family_id),
                (fm.user_id = t.user_id OR fm.role = 'family_admin')
         FROM family_members fm
         CROSS JOIN {table} t
         WHERE fm.family_id = $1 AND t.id = $3
         ORDER BY fm.joined_at"
    );

    sqlx::query_as::<_, (Uuid, bool, bool)>(&sql)
        .bind(family_id)
        .bind(kind)
        .bind(item_id)
        .fetch_all(pool)
        .await
        .context("db: item audience")
}

// ── Content reports (migration 0057) ─────────────────────────────────────

pub const REPORT_REASONS: [&str; 4] = ["offensive", "inaccurate", "not_for_children", "other"];

pub fn is_valid_report_reason(reason: &str) -> bool {
    REPORT_REASONS.contains(&reason)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContentReport {
    pub id: Uuid,
    pub reporter_user_id: Option<Uuid>,
    pub reporter_name: Option<String>,
    pub media_kind: String,
    pub item_id: Uuid,
    pub reason: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

pub async fn insert_report(
    pool: &PgPool,
    family_id: Uuid,
    reporter_user_id: Uuid,
    media_kind: &str,
    item_id: Uuid,
    reason: &str,
    note: Option<&str>,
) -> anyhow::Result<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO content_reports
             (family_id, reporter_user_id, media_kind, item_id, reason, note)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(family_id)
    .bind(reporter_user_id)
    .bind(media_kind)
    .bind(item_id)
    .bind(reason)
    .bind(note)
    .fetch_one(pool)
    .await
    .context("db: insert content report")
}

/// Open reports for a family, newest first. Resolved ones stay in the table as
/// a record but are not surfaced.
pub async fn list_open_reports(
    pool: &PgPool,
    family_id: Uuid,
) -> anyhow::Result<Vec<ContentReport>> {
    sqlx::query_as::<_, ContentReport>(
        "SELECT r.id, r.reporter_user_id, u.display_name AS reporter_name,
                r.media_kind, r.item_id, r.reason, r.note,
                r.created_at, r.resolved_at
         FROM content_reports r
         LEFT JOIN users u ON u.id = r.reporter_user_id
         WHERE r.family_id = $1 AND r.resolved_at IS NULL
         ORDER BY r.created_at DESC",
    )
    .bind(family_id)
    .fetch_all(pool)
    .await
    .context("db: list open content reports")
}

pub async fn resolve_report(
    pool: &PgPool,
    family_id: Uuid,
    report_id: Uuid,
    resolved_by: Uuid,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE content_reports
         SET resolved_at = now(), resolved_by = $3
         WHERE id = $2 AND family_id = $1 AND resolved_at IS NULL",
    )
    .bind(family_id)
    .bind(report_id)
    .bind(resolved_by)
    .execute(pool)
    .await
    .context("db: resolve content report")?;
    Ok(result.rows_affected() > 0)
}
