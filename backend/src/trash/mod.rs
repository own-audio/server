// SPDX-License-Identifier: AGPL-3.0-or-later
//! The trash API — docs/file-sync-plan.md §5.1.
//!
//! Every delete route moves its item here through [`move_to_trash`]; this
//! module owns listing, restoring (which charges the days in the trash) and
//! purging. The purge job lives in `jobs::worker` and calls [`purge`].
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::db::access::Viewer;
use crate::db::trash::{Kind, Scope, Trashed};
use crate::families::FamilyContext;
use crate::storage::ObjectStore;
use sqlx::PgPool;

/// Clients send one id for everything deleted in one gesture, so the trash
/// can restore the whole deletion at once. Without it each delete is its own
/// batch.
pub const BATCH_HEADER: &str = "x-trash-batch";

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_trash))
        .routes(routes!(empty_trash))
        .routes(routes!(restore_batch))
        .routes(routes!(restore_one))
        .routes(routes!(purge_one))
}

fn batch_from(headers: &HeaderMap) -> Uuid {
    headers
        .get(BATCH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v.trim()).ok())
        .unwrap_or_else(Uuid::new_v4)
}

/// Owner, or a family admin for something shared with their family. The same
/// rule decides who may trash, restore and purge.
fn may_manage(viewer: Viewer, owner_id: Uuid, item_family: Option<Uuid>) -> bool {
    owner_id == viewer.user_id || (viewer.is_family_admin && item_family == Some(viewer.family_id))
}

/// Move one live item to the trash on behalf of `viewer`. Used by every delete
/// route, the Subsonic one included. Errors follow the 404 rule: someone who
/// cannot see the item gets `ItemNotFound`; someone who can see it but may not
/// delete it gets `Forbidden`.
pub async fn move_to_trash(
    state: &AppState,
    viewer: Viewer,
    headers: Option<&HeaderMap>,
    kind: Kind,
    id: Uuid,
) -> Result<(), AuthError> {
    let pool = state.db();
    let target = db::trash::find_live(pool, kind, id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if !may_manage(viewer, target.owner_id, target.family_id) {
        let visible = match kind.access_kind() {
            Some(access_kind) => db::access::can_listen(pool, viewer, access_kind, target.access_id)
                .await
                .map_err(AuthError::Internal)?,
            None => target.family_id == Some(viewer.family_id),
        };
        return Err(if visible { AuthError::Forbidden } else { AuthError::ItemNotFound });
    }

    let batch = headers.map(batch_from).unwrap_or_else(Uuid::new_v4);
    if !db::trash::move_to_trash(pool, kind, id, viewer.user_id, batch)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::ItemNotFound);
    }

    if let Some(tombstone_kind) = kind.tombstone_kind() {
        let _ = db::sync::record_deletion(pool, tombstone_kind, id, target.owner_id, target.family_id).await;
    }

    if target.owner_id != viewer.user_id {
        let actor = db::users::find_by_id(pool, viewer.user_id)
            .await
            .ok()
            .flatten()
            .map(|u| u.display_name)
            .unwrap_or_default();
        let purge_at = Utc::now() + chrono::Duration::days(db::trash::RETENTION_DAYS);
        let data = json!({
            "kind": kind.as_str(),
            "id": id,
            "title": target.title,
            "by": viewer.user_id,
            "purge_at": purge_at.to_rfc3339(),
        });
        let _ = db::sync::queue_notification(
            pool,
            target.owner_id,
            "item_trashed",
            &format!("“{}” was moved to the trash", target.title),
            Some(&format!("{actor} moved it. You can restore it for 30 days.")),
            Some(&data),
        )
        .await;
    }

    Ok(())
}

/// Delete a trashed item for good, with its stored objects where nothing else
/// references them. Returns false when it was no longer in the trash.
pub async fn purge(pool: &PgPool, storage: &ObjectStore, kind: Kind, id: Uuid) -> anyhow::Result<bool> {
    let Some(objects) = db::trash::purge(pool, kind, id).await? else {
        return Ok(false);
    };
    delete_orphans(pool, storage, &objects).await;
    if let Some(k) = path_kind(kind) {
        // A book or track row is gone and its path went with it (migration
        // 0078). An episode row stays, so its path goes here — unless the
        // episode was stored again meanwhile and the path is the new copy's.
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sync_live_paths WHERE kind = $1 AND item_id = $2)",
        )
        .bind(k.as_str())
        .bind(id)
        .fetch_one(pool)
        .await?;
        if !live {
            sqlx::query("DELETE FROM sync_paths WHERE kind = $1 AND item_id = $2")
                .bind(k.as_str())
                .bind(id)
                .execute(pool)
                .await?;
            crate::filesync::holdings::forget_item(pool, k.as_str(), id).await?;
        }
    }
    Ok(true)
}

/// The kinds that sit in the own.audio folder.
fn path_kind(kind: Kind) -> Option<crate::filesync::paths::Kind> {
    use crate::filesync::paths::Kind as P;
    match kind {
        Kind::Audiobook => Some(P::Audiobook),
        Kind::MusicTrack => Some(P::MusicTrack),
        Kind::PodcastEpisode => Some(P::PodcastEpisode),
        Kind::CompanionFile => Some(P::CompanionFile),
        Kind::Playlist => None,
    }
}

/// Delete each object that nothing references any more. Best-effort: a failure
/// leaves the object for the storage sweep.
pub async fn delete_orphans(pool: &PgPool, storage: &ObjectStore, objects: &[Uuid]) {
    for object_id in objects {
        match db::media::delete_object_if_unreferenced(pool, *object_id).await {
            Ok(Some(key)) => {
                if let Err(e) = storage.delete(&key).await {
                    tracing::warn!(%object_id, %key, error = %e, "row gone but the object stayed in storage");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(%object_id, error = %e, "could not check whether the object is still referenced"),
        }
    }
}

// ── Listing ───────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct ListQuery {
    /// `mine` (the default) or `family`; `family` needs a family admin.
    scope: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[schema(as = TrashPerson)]
struct Person {
    id: Uuid,
    display_name: Option<String>,
}

#[derive(Serialize, ToSchema)]
struct TrashItemResponse {
    /// `audiobook`, `music_track`, `playlist`, `podcast_episode` or `companion_file`.
    kind: &'static str,
    id: Uuid,
    title: String,
    owner: Person,
    trashed_by: Option<Person>,
    trashed_at: DateTime<Utc>,
    purge_at: DateTime<Utc>,
    size_bytes: i64,
    batch: Option<Uuid>,
    /// What restoring it right now would charge, so a client can say so
    /// before the user taps Restore. Zero when charges are off.
    restore_charge_micro: i64,
}

/// What restoring this item now would cost — the edition's call (`Hooks`);
/// nothing in the open-source edition.
async fn restore_quote(state: &AppState, size_bytes: i64, trashed_at: DateTime<Utc>) -> Result<i64, AuthError> {
    let days = db::trash::days_in_trash(trashed_at, Utc::now());
    state
        .hooks()
        .trash_restore_quote(state.db(), size_bytes, days)
        .await
        .map_err(AuthError::Internal)
}

async fn to_response(state: &AppState, t: Trashed) -> Result<TrashItemResponse, AuthError> {
    let restore_charge_micro = restore_quote(state, t.size_bytes, t.trashed_at).await?;
    let purge_at = t.purge_at();
    Ok(TrashItemResponse {
        kind: t.kind().as_str(),
        id: t.id,
        title: t.title,
        owner: Person { id: t.owner_id, display_name: t.owner_name },
        trashed_by: t.trashed_by.map(|id| Person { id, display_name: t.trashed_by_name }),
        trashed_at: t.trashed_at,
        purge_at,
        size_bytes: t.size_bytes,
        batch: t.trash_batch,
        restore_charge_micro,
    })
}

fn scope_from(family: &FamilyContext, q: &ListQuery) -> Result<Scope, AuthError> {
    match q.scope.as_deref() {
        None | Some("mine") => Ok(Scope::Owner(family.user_id)),
        Some("family") => {
            family.require_family_admin()?;
            Ok(Scope::Family(family.family_id))
        }
        Some(_) => Err(AuthError::BadRequest("scope must be 'mine' or 'family'".into())),
    }
}

/// GET /api/v1/trash?scope=mine|family
///
/// What is in the trash: the caller's own items, or the whole family's.
#[utoipa::path(get, path = "/", tag = "trash", security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = Vec<TrashItemResponse>),
        (status = 400, description = "Unknown scope", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "`family` scope without being a family admin", body = crate::http::openapi::ErrorBody)))]
async fn list_trash(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<TrashItemResponse>>, AuthError> {
    let scope = scope_from(&family, &q)?;
    let items = db::trash::list(state.db(), scope).await.map_err(AuthError::Internal)?;
    let mut out = Vec::with_capacity(items.len());
    for t in items {
        out.push(to_response(&state, t).await?);
    }
    Ok(Json(out))
}

// ── Restoring ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
#[schema(as = TrashRestoreResponse)]
struct RestoreResponse {
    restored: usize,
    charged_micro: i64,
}

/// Restore one trashed item the caller may manage, charging its days in the
/// trash to the owner's family. `None` when it was not in the trash any more.
async fn restore_item(
    state: &AppState,
    family: &FamilyContext,
    item: &Trashed,
) -> Result<Option<i64>, AuthError> {
    let pool = state.db();
    let kind = item.kind();
    let Some(surplus) = db::trash::restore(pool, kind, item.id).await.map_err(AuthError::Internal)? else {
        return Ok(None);
    };
    delete_orphans(pool, state.storage(), &surplus).await;
    if let Some(k) = path_kind(kind) {
        crate::filesync::paths::settle_after_restore(pool, k, item.id).await.map_err(AuthError::Internal)?;
    }

    let days = db::trash::days_in_trash(item.trashed_at, Utc::now());
    let Some(owner_family) = db::families::find_membership(pool, item.owner_id)
        .await
        .map_err(AuthError::Internal)?
        .map(|m| m.family_id)
    else {
        return Ok(Some(0));
    };

    // Whether and how much this costs is the edition's call (`Hooks`); the
    // open-source edition charges nothing and returns 0.
    let note = format!(
        "Restored “{}” after {days} day{} in the trash",
        item.title,
        if days == 1 { "" } else { "s" }
    );
    let charged = state
        .hooks()
        .trash_restore_charge(pool, owner_family, family.user_id, item.size_bytes, days, &note)
        .await
        .map_err(AuthError::Internal)?;
    let _ = db::trash::record_restore(
        pool,
        owner_family,
        family.user_id,
        kind,
        item.id,
        item.size_bytes,
        days,
        charged,
    )
    .await;
    Ok(Some(charged))
}

fn parse_kind(kind: &str) -> Result<Kind, AuthError> {
    Kind::parse(kind).ok_or_else(|| {
        AuthError::BadRequest("kind must be 'audiobook', 'music_track', 'playlist' or 'podcast_episode'".into())
    })
}

/// A trashed item the caller may manage — anything else is 404, including
/// someone else's trash: it is not visible to them.
async fn managed_trashed(state: &AppState, family: &FamilyContext, kind: Kind, id: Uuid) -> Result<Trashed, AuthError> {
    let item = db::trash::find_trashed(state.db(), kind, id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    if !may_manage(family.viewer(), item.owner_id, item.family_id) {
        return Err(AuthError::ItemNotFound);
    }
    Ok(item)
}

/// POST /api/v1/trash/{kind}/{id}/restore
///
/// Restore one item, charging its days in the trash to the owner's family.
#[utoipa::path(post, path = "/{kind}/{id}/restore", tag = "trash", security(("bearer" = [])),
    params(("kind" = String, Path, description = "`audiobook`, `music_track`, `playlist`, `podcast_episode` or `companion_file`"), ("id" = Uuid, Path, description = "Item id")),
    responses((status = 200, body = RestoreResponse),
        (status = 400, description = "Unknown kind", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "Not in the trash, or not the caller's to restore", body = crate::http::openapi::ErrorBody)))]
async fn restore_one(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, Uuid)>,
) -> Result<Json<RestoreResponse>, AuthError> {
    let item = managed_trashed(&state, &family, parse_kind(&kind)?, id).await?;
    let charged = restore_item(&state, &family, &item).await?.ok_or(AuthError::ItemNotFound)?;
    Ok(Json(RestoreResponse { restored: 1, charged_micro: charged }))
}

/// POST /api/v1/trash/batches/{batch}/restore — everything deleted together,
/// as far as the caller may restore it.
#[utoipa::path(post, path = "/batches/{batch}/restore", tag = "trash", security(("bearer" = [])),
    params(("batch" = Uuid, Path, description = "The `x-trash-batch` id the items were deleted with")),
    responses((status = 200, body = RestoreResponse),
        (status = 404, description = "Nothing in the batch the caller may restore", body = crate::http::openapi::ErrorBody)))]
async fn restore_batch(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(batch): Path<Uuid>,
) -> Result<Json<RestoreResponse>, AuthError> {
    let items = db::trash::list_batch(state.db(), batch).await.map_err(AuthError::Internal)?;
    let mut restored = 0;
    let mut charged_micro = 0;
    for item in items.iter().filter(|i| may_manage(family.viewer(), i.owner_id, i.family_id)) {
        if let Some(charged) = restore_item(&state, &family, item).await? {
            restored += 1;
            charged_micro += charged;
        }
    }
    if restored == 0 {
        return Err(AuthError::ItemNotFound);
    }
    Ok(Json(RestoreResponse { restored, charged_micro }))
}

// ── Deleting for good ─────────────────────────────────────────────────────

/// DELETE /api/v1/trash/{kind}/{id} — delete forever, now.
#[utoipa::path(delete, path = "/{kind}/{id}", tag = "trash", security(("bearer" = [])),
    params(("kind" = String, Path, description = "`audiobook`, `music_track`, `playlist`, `podcast_episode` or `companion_file`"), ("id" = Uuid, Path, description = "Item id")),
    responses((status = 204, description = "Deleted for good"),
        (status = 400, description = "Unknown kind", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "Not in the trash, or not the caller's to delete", body = crate::http::openapi::ErrorBody)))]
async fn purge_one(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AuthError> {
    let kind = parse_kind(&kind)?;
    managed_trashed(&state, &family, kind, id).await?;
    if !purge(state.db(), state.storage(), kind, id).await.map_err(AuthError::Internal)? {
        return Err(AuthError::ItemNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[schema(as = TrashEmptyResponse)]
struct EmptyResponse {
    purged: usize,
}

/// POST /api/v1/trash/empty?scope=mine|family
///
/// Delete for good everything in the trash the caller may manage.
#[utoipa::path(post, path = "/empty", tag = "trash", security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = EmptyResponse),
        (status = 400, description = "Unknown scope", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "`family` scope without being a family admin", body = crate::http::openapi::ErrorBody)))]
async fn empty_trash(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<EmptyResponse>, AuthError> {
    let scope = scope_from(&family, &q)?;
    let items = db::trash::list(state.db(), scope).await.map_err(AuthError::Internal)?;
    let mut purged = 0;
    for item in items.iter().filter(|i| may_manage(family.viewer(), i.owner_id, i.family_id)) {
        if purge(state.db(), state.storage(), item.kind(), item.id).await.map_err(AuthError::Internal)? {
            purged += 1;
        }
    }
    Ok(Json(EmptyResponse { purged }))
}
