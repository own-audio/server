// SPDX-License-Identifier: AGPL-3.0-or-later
/// Family (user group) persistence: membership, roles, invites.
/// See migration 0018. Every user belongs to exactly one family; solo users
/// get a personal family of one so callers never branch on "has a family".
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Family {
    pub id: Uuid,
    pub name: String,
    pub created_by: Option<Uuid>,
    /// The family's own photo (distinct from any member's personal avatar,
    /// migration 0043) — set via `POST /family/avatar`, family_admin only.
    pub avatar_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A member joined with their user record, for the members list.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FamilyMember {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub display_label: Option<String>,
    pub role: String,
    pub is_active: bool,
    /// True for a provisioned account nobody has claimed yet — it has no
    /// auth identity, so nobody can sign in as it.
    pub pending: bool,
    pub avatar_object_id: Option<Uuid>,
    pub joined_at: DateTime<Utc>,
    /// `adult` | `teen` | `child`. Set by an admin, never by the member — see
    /// docs/family-permissions-plan.md.
    pub age_bracket: String,
    pub can_upload: bool,
    pub can_generate: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FamilyInvite {
    pub id: Uuid,
    pub family_id: Uuid,
    /// `email` kind only; `link` and `claim` invites are not address-bound.
    pub email: Option<String>,
    pub code: String,
    pub role: String,
    /// `email` | `link` | `claim` — see migration 0042.
    pub kind: String,
    pub max_uses: i32,
    pub use_count: i32,
    /// Admin-facing note ("fridge QR"), never shown to the invitee.
    pub label: Option<String>,
    /// `claim` kind: the pre-provisioned account this code activates.
    pub claim_user_id: Option<Uuid>,
    /// Who created the invite — shown on the join preview as "X invited you".
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
}

impl FamilyInvite {
    pub fn exhausted(&self) -> bool {
        self.use_count >= self.max_uses
    }

    pub fn expired(&self) -> bool {
        self.expires_at <= Utc::now()
    }
}

/// Column list shared by every invite query; keep in sync with the struct.
const INVITE_COLUMNS: &str = "id, family_id, email, code, role, kind, max_uses, use_count, \
                              label, claim_user_id, created_by, created_at, expires_at, accepted_at";

// ── Membership ────────────────────────────────────────────────────────────

/// The caller's family id + role, if they have a membership row.
/// What the caller may do in their family. A struct rather than a widening
/// tuple: every consumer wants a different subset, and `FamilyContext` builds
/// its permission checks straight off it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Membership {
    pub family_id: Uuid,
    pub role: String,
    pub age_bracket: String,
    pub can_upload: bool,
    pub can_generate: bool,
}

pub async fn find_membership(
    pool: &PgPool,
    user_id: Uuid,
) -> anyhow::Result<Option<Membership>> {
    sqlx::query_as::<_, Membership>(
        "SELECT family_id, role, age_bracket, can_upload, can_generate
         FROM family_members WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find family membership")
}

/// Resolve the user's family, giving them one ([`home_new_user`]) if they have
/// none (users created before this feature, or by a path that forgot to call
/// [`create_personal_family`]). Idempotent.
pub async fn ensure_membership(pool: &PgPool, user_id: Uuid, one_family: bool) -> anyhow::Result<Membership> {
    if let Some(found) = find_membership(pool, user_id).await? {
        return Ok(found);
    }
    home_new_user(pool, user_id, one_family).await
}

/// Give a brand-new user their family. With `one_family` (the open-source
/// edition) everyone joins the install's family as a member, and only the
/// very first user founds it; otherwise each user gets a family of their own.
pub async fn home_new_user(pool: &PgPool, user_id: Uuid, one_family: bool) -> anyhow::Result<Membership> {
    if !one_family {
        return create_personal_family(pool, user_id).await;
    }
    let mut tx = pool.begin().await.context("db: begin home new user")?;
    // Two first sign-ups at the same moment must not found two families.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('own-audio:one-family'))")
        .execute(&mut *tx)
        .await
        .context("db: lock family founding")?;
    let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM families ORDER BY created_at, id LIMIT 1")
        .fetch_optional(&mut *tx)
        .await
        .context("db: find the install's family")?;
    let (family_id, role) = match existing {
        Some(id) => (id, "member"),
        None => {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO families (name, created_by)
                 SELECT display_name, id FROM users WHERE id = $1 RETURNING id",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .context("db: found the install's family")?;
            (id, "family_admin")
        }
    };
    sqlx::query("INSERT INTO family_members (family_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(family_id)
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await
        .context("db: join the install's family")?;
    tx.commit().await.context("db: commit home new user")?;
    find_membership(pool, user_id)
        .await?
        .context("db: membership just created is missing")
}

/// Create a personal family of one with `user_id` as its family_admin.
/// Names the family after the user's display name.
pub async fn create_personal_family(
    pool: &PgPool,
    user_id: Uuid,
) -> anyhow::Result<Membership> {
    let mut tx = pool.begin().await.context("db: begin personal family")?;

    let display_name: String =
        sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .context("db: load user for personal family")?;

    let family_id: Uuid = sqlx::query_scalar(
        "INSERT INTO families (name, created_by) VALUES ($1, $2) RETURNING id",
    )
    .bind(&display_name)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .context("db: insert personal family")?;

    sqlx::query(
        "INSERT INTO family_members (family_id, user_id, role)
         VALUES ($1, $2, 'family_admin')
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(family_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .context("db: insert personal family member")?;

    tx.commit().await.context("db: commit personal family")?;

    // A concurrent request may have won the ON CONFLICT race; return whatever
    // membership actually persisted.
    let membership = find_membership(pool, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("personal family vanished after insert"))?;

    Ok(membership)
}

/// One row per family, for the instance-admin families list
/// (`GET /admin/families`) — every family on the server, not just the
/// caller's own.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FamilySummary {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub member_count: i64,
}

pub async fn list_all(pool: &PgPool) -> anyhow::Result<Vec<FamilySummary>> {
    sqlx::query_as::<_, FamilySummary>(
        "SELECT f.id, f.name, f.created_at, COUNT(fm.user_id) AS member_count
         FROM families f
         LEFT JOIN family_members fm ON fm.family_id = f.id
         GROUP BY f.id
         ORDER BY f.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("db: list all families")
}

pub async fn find(pool: &PgPool, family_id: Uuid) -> anyhow::Result<Option<Family>> {
    sqlx::query_as::<_, Family>(
        "SELECT id, name, created_by, avatar_object_id, created_at, updated_at
         FROM families WHERE id = $1",
    )
    .bind(family_id)
    .fetch_optional(pool)
    .await
    .context("db: find family")
}

pub async fn rename(pool: &PgPool, family_id: Uuid, name: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE families SET name = $2, updated_at = now() WHERE id = $1")
        .bind(family_id)
        .bind(name)
        .execute(pool)
        .await
        .context("db: rename family")?;
    Ok(())
}

/// Sets or clears (`media_id: None`) the family's own photo. Generic over the executor — see
/// `db::users::set_avatar`'s doc comment on why (run it in the same transaction as the
/// `media_objects` insert it points at).
pub async fn set_avatar<'e, E>(executor: E, family_id: Uuid, media_id: Option<Uuid>) -> anyhow::Result<()>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query("UPDATE families SET avatar_object_id = $2, updated_at = now() WHERE id = $1")
        .bind(family_id)
        .bind(media_id)
        .execute(executor)
        .await
        .context("db: set family avatar")?;
    Ok(())
}

/// The family photo's storage key + content type — `None` if unset. Callers gate visibility
/// (family membership) before calling this, same as `db::users::find_avatar`.
pub async fn find_avatar(pool: &PgPool, family_id: Uuid) -> anyhow::Result<Option<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT mo.object_key, mo.content_type
         FROM media_objects mo
         JOIN families f ON f.avatar_object_id = mo.id
         WHERE f.id = $1",
    )
    .bind(family_id)
    .fetch_optional(pool)
    .await
    .context("db: find family avatar")
}

/// Deletes the family iff it currently has zero members. Generic over the
/// executor (see `set_avatar`'s doc comment on why) so both
/// `move_to_personal_family`/`move_to_family` can call it inside their own
/// transaction and a plain caller (an instance admin removing an account, or
/// deleting an empty family directly) can call it against the pool. Returns
/// whether it actually deleted anything — false means either the family
/// still has members, or never existed.
pub async fn delete_if_empty<'e, E>(executor: E, family_id: Uuid) -> anyhow::Result<bool>
where
    E: sqlx::PgExecutor<'e>,
{
    let result = sqlx::query(
        "DELETE FROM families f
         WHERE f.id = $1
           AND NOT EXISTS (SELECT 1 FROM family_members m WHERE m.family_id = f.id)",
    )
    .bind(family_id)
    .execute(executor)
    .await
    .context("db: delete family if empty")?;
    Ok(result.rows_affected() == 1)
}

/// Every family with zero members right now — candidates for the periodic
/// cleanup sweep. This is a safety net only: every normal path that can
/// empty a family (`move_to_family`, `move_to_personal_family`,
/// `delete_self`, `admin_delete_user`) already prunes inline via
/// `delete_if_empty`; this exists to catch whatever a future bug in one of
/// those misses, not as the primary mechanism.
pub async fn find_empty(pool: &PgPool) -> anyhow::Result<Vec<(Uuid, String)>> {
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT f.id, f.name FROM families f
         WHERE NOT EXISTS (SELECT 1 FROM family_members m WHERE m.family_id = f.id)",
    )
    .fetch_all(pool)
    .await
    .context("db: find empty families")
}

pub async fn list_members(pool: &PgPool, family_id: Uuid) -> anyhow::Result<Vec<FamilyMember>> {
    sqlx::query_as::<_, FamilyMember>(
        "SELECT fm.user_id, u.email, u.display_name, fm.display_label,
                fm.role, u.is_active,
                NOT EXISTS (SELECT 1 FROM auth_identities ai WHERE ai.user_id = u.id)
                    AS pending,
                u.avatar_object_id,
                fm.joined_at,
                fm.age_bracket, fm.can_upload, fm.can_generate
         FROM family_members fm
         JOIN users u ON u.id = fm.user_id
         WHERE fm.family_id = $1
         ORDER BY fm.joined_at",
    )
    .bind(family_id)
    .fetch_all(pool)
    .await
    .context("db: list family members")
}

pub async fn count_admins(pool: &PgPool, family_id: Uuid) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM family_members
         WHERE family_id = $1 AND role = 'family_admin'",
    )
    .bind(family_id)
    .fetch_one(pool)
    .await
    .context("db: count family admins")
}

pub async fn set_role(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE family_members SET role = $3 WHERE family_id = $1 AND user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(role)
    .execute(pool)
    .await
    .context("db: set family member role")?;

    Ok(result.rows_affected() == 1)
}

pub async fn set_member_permissions(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    age_bracket: &str,
    can_upload: bool,
    can_generate: bool,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE family_members
         SET age_bracket = $3, can_upload = $4, can_generate = $5
         WHERE family_id = $1 AND user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(age_bracket)
    .bind(can_upload)
    .bind(can_generate)
    .execute(pool)
    .await
    .context("db: set member permissions")?;
    Ok(result.rows_affected() > 0)
}

pub async fn set_display_label(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    label: Option<&str>,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE family_members SET display_label = $3 WHERE family_id = $1 AND user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(label)
    .execute(pool)
    .await
    .context("db: set family member label")?;

    Ok(result.rows_affected() == 1)
}

/// Move a user out of their current family into a brand-new personal family
/// where they are the family_admin. Used when a member is removed or leaves.
pub async fn move_to_personal_family(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Uuid> {
    let mut tx = pool.begin().await.context("db: begin personal family move")?;

    let display_name: String =
        sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .context("db: load user for personal family move")?;

    let previous: Option<Uuid> =
        sqlx::query_scalar("SELECT family_id FROM family_members WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await
            .context("db: load previous family")?;

    let family_id: Uuid = sqlx::query_scalar(
        "INSERT INTO families (name, created_by) VALUES ($1, $2) RETURNING id",
    )
    .bind(&display_name)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .context("db: insert personal family")?;

    sqlx::query("DELETE FROM family_members WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .context("db: clear previous membership")?;

    sqlx::query(
        "INSERT INTO family_members (family_id, user_id, role)
         VALUES ($1, $2, 'family_admin')",
    )
    .bind(family_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .context("db: insert personal membership")?;

    if let Some(previous) = previous {
        delete_if_empty(&mut *tx, previous).await?;
    }

    tx.commit()
        .await
        .context("db: commit personal family move")?;

    Ok(family_id)
}

/// Move a user into `family_id` with `role`, dropping any previous membership
/// and deleting the family they left if it is now empty.
///
/// NOTE (D1, Phase 2): content re-parenting rides along here once content
/// tables carry `family_id` — a joiner brings their library with them.
pub async fn move_to_family(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Uuid,
    role: &str,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin family move")?;

    let previous: Option<Uuid> =
        sqlx::query_scalar("SELECT family_id FROM family_members WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await
            .context("db: load previous family")?;

    sqlx::query("DELETE FROM family_members WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .context("db: clear previous membership")?;

    sqlx::query("INSERT INTO family_members (family_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(family_id)
        .bind(user_id)
        .bind(role)
        .execute(&mut *tx)
        .await
        .context("db: insert new membership")?;

    // Clean up a family the user just emptied (typically their personal one).
    if let Some(previous) = previous.filter(|p| *p != family_id) {
        delete_if_empty(&mut *tx, previous).await?;
    }

    tx.commit().await.context("db: commit family move")
}

/// Add a brand-new user straight into a family (provisioned accounts — the
/// user was created moments ago and has no membership to move from).
pub async fn add_member(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO family_members (family_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(family_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .context("db: add family member")?;
    Ok(())
}

// ── Invites ───────────────────────────────────────────────────────────────

/// Everything needed to insert an invite; the handler layer enforces the
/// kind/role/uses combinations before this runs (the DB CHECK is the backstop).
pub struct NewInvite<'a> {
    pub family_id: Uuid,
    pub email: Option<&'a str>,
    pub code: &'a str,
    pub role: &'a str,
    pub kind: &'a str,
    pub max_uses: i32,
    pub label: Option<&'a str>,
    pub claim_user_id: Option<Uuid>,
    pub created_by: Uuid,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_invite(pool: &PgPool, new: NewInvite<'_>) -> anyhow::Result<FamilyInvite> {
    sqlx::query_as::<_, FamilyInvite>(&format!(
        "INSERT INTO family_invites
             (family_id, email, code, role, kind, max_uses, label, claim_user_id,
              created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING {INVITE_COLUMNS}",
    ))
    .bind(new.family_id)
    .bind(new.email)
    .bind(new.code)
    .bind(new.role)
    .bind(new.kind)
    .bind(new.max_uses)
    .bind(new.label)
    .bind(new.claim_user_id)
    .bind(new.created_by)
    .bind(new.expires_at)
    .fetch_one(pool)
    .await
    .context("db: create family invite")
}

/// Pending = not yet exhausted. Expired invites still list so the admin can
/// see and regenerate them rather than wondering where they went.
pub async fn list_invites(pool: &PgPool, family_id: Uuid) -> anyhow::Result<Vec<FamilyInvite>> {
    sqlx::query_as::<_, FamilyInvite>(&format!(
        "SELECT {INVITE_COLUMNS}
         FROM family_invites
         WHERE family_id = $1 AND use_count < max_uses
         ORDER BY created_at DESC",
    ))
    .bind(family_id)
    .fetch_all(pool)
    .await
    .context("db: list family invites")
}

pub async fn find_invite_by_code(
    pool: &PgPool,
    code: &str,
) -> anyhow::Result<Option<FamilyInvite>> {
    sqlx::query_as::<_, FamilyInvite>(&format!(
        "SELECT {INVITE_COLUMNS} FROM family_invites WHERE code = $1",
    ))
    .bind(code)
    .fetch_optional(pool)
    .await
    .context("db: find family invite")
}

pub async fn find_invite_by_id(
    pool: &PgPool,
    id: Uuid,
    family_id: Uuid,
) -> anyhow::Result<Option<FamilyInvite>> {
    sqlx::query_as::<_, FamilyInvite>(&format!(
        "SELECT {INVITE_COLUMNS} FROM family_invites WHERE id = $1 AND family_id = $2",
    ))
    .bind(id)
    .bind(family_id)
    .fetch_optional(pool)
    .await
    .context("db: find family invite by id")
}

/// Give a pending invite a fresh code and TTL, invalidating the old code.
/// Returns None when the invite is already exhausted (nothing to renew).
pub async fn regenerate_invite(
    pool: &PgPool,
    id: Uuid,
    family_id: Uuid,
    code: &str,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<Option<FamilyInvite>> {
    sqlx::query_as::<_, FamilyInvite>(&format!(
        "UPDATE family_invites
         SET code = $3, expires_at = $4
         WHERE id = $1 AND family_id = $2 AND use_count < max_uses
         RETURNING {INVITE_COLUMNS}",
    ))
    .bind(id)
    .bind(family_id)
    .bind(code)
    .bind(expires_at)
    .fetch_optional(pool)
    .await
    .context("db: regenerate family invite")
}

/// The pending claim invite for a provisioned account, if any.
pub async fn find_claim_invite_for_user(
    pool: &PgPool,
    claim_user_id: Uuid,
) -> anyhow::Result<Option<FamilyInvite>> {
    sqlx::query_as::<_, FamilyInvite>(&format!(
        "SELECT {INVITE_COLUMNS}
         FROM family_invites
         WHERE claim_user_id = $1 AND use_count < max_uses
         ORDER BY created_at DESC LIMIT 1",
    ))
    .bind(claim_user_id)
    .fetch_optional(pool)
    .await
    .context("db: find claim invite for user")
}

pub async fn delete_invite(pool: &PgPool, id: Uuid, family_id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM family_invites WHERE id = $1 AND family_id = $2")
        .bind(id)
        .bind(family_id)
        .execute(pool)
        .await
        .context("db: delete family invite")?;

    Ok(result.rows_affected() == 1)
}

/// Consume one use of an invite. Returns false when it is exhausted or
/// expired — the caller must treat that as "invite no longer valid" rather
/// than retrying. The single UPDATE is the atomic guard against two devices
/// redeeming the last use concurrently.
pub async fn mark_invite_accepted(
    pool: &PgPool,
    id: Uuid,
    accepted_by: Uuid,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE family_invites
         SET use_count = use_count + 1, accepted_at = now(), accepted_by = $2
         WHERE id = $1 AND use_count < max_uses AND expires_at > now()",
    )
    .bind(id)
    .bind(accepted_by)
    .execute(pool)
    .await
    .context("db: accept family invite")?;

    Ok(result.rows_affected() == 1)
}
