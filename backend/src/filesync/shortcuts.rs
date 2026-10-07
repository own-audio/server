// SPDX-License-Identifier: AGPL-3.0-or-later
/// Family shortcuts — docs/file-sync-plan.md §4.4 and §5.5.
///
/// "Add to own.audio folder" on a member's whole kind or on one book, album or
/// show. Shortcuts belong to the account, so they appear on all its devices.
/// The server only stores them; the sync client decides from them which
/// family items go into `Family/`, so the rule lives in one place.
use crate::db::access::{self, Viewer};
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Shortcut {
    pub id: Uuid,
    pub member_id: Uuid,
    pub member_name: Option<String>,
    /// `audiobook`, `music` or `podcast`.
    pub kind: String,
    /// `book`, `album`, `show`, or null for the member's whole kind.
    pub container_kind: Option<String>,
    /// The book or show.
    pub container_id: Option<Uuid>,
    /// An identified album.
    pub release_group: Option<String>,
    /// An unidentified album.
    pub album_artist: Option<String>,
    pub album: Option<String>,
    /// What the app shows: the book, album or show title, or null for a kind.
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct NewShortcut {
    /// Optional with a container: the book's, show's or album's owner is found here.
    #[serde(default)]
    pub member_id: Option<Uuid>,
    pub kind: String,
    #[serde(default)]
    pub container: Option<Container>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = ShortcutContainer)]
pub struct Container {
    /// `book`, `album` or `show`.
    pub kind: String,
    /// The book or show id.
    #[serde(default)]
    pub id: Option<Uuid>,
    #[serde(default)]
    pub release_group: Option<String>,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
}

const SELECT: &str = "SELECT s.id, s.member_id,
            (SELECT COALESCE(fm.display_label, u.display_name)
               FROM users u LEFT JOIN family_members fm ON fm.user_id = u.id
              WHERE u.id = s.member_id) AS member_name,
            s.kind, s.container_kind, s.container_id, s.release_group, s.album_artist, s.album,
            CASE s.container_kind
                WHEN 'book' THEN (SELECT b.title FROM audiobook_books b WHERE b.id = s.container_id)
                WHEN 'show' THEN (SELECT f.title FROM podcast_feeds f WHERE f.id = s.container_id)
                WHEN 'album' THEN COALESCE(s.album,
                    (SELECT t.album FROM music_tracks t
                      WHERE t.user_id = s.member_id AND t.musicbrainz_release_group_id = s.release_group
                      LIMIT 1))
            END AS label,
            s.created_at
       FROM sync_shortcuts s";

pub async fn list(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<Shortcut>> {
    sqlx::query_as::<_, Shortcut>(&format!("{SELECT} WHERE s.user_id = $1 ORDER BY s.created_at"))
        .bind(user_id)
        .fetch_all(pool)
        .await
        .context("db: list shortcuts")
}

pub async fn find(pool: &PgPool, user_id: Uuid, id: Uuid) -> anyhow::Result<Option<Shortcut>> {
    sqlx::query_as::<_, Shortcut>(&format!("{SELECT} WHERE s.user_id = $1 AND s.id = $2"))
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find shortcut")
}

/// Why a shortcut cannot be made.
#[derive(Debug, PartialEq)]
pub enum Refusal {
    Invalid(&'static str),
    /// Not a member of the caller's family, or nothing the caller can see.
    NotFound,
}

/// Add a shortcut. Adding one that exists returns the existing one and
/// `false`.
pub async fn add(pool: &PgPool, viewer: Viewer, body: NewShortcut) -> anyhow::Result<Result<(Shortcut, bool), Refusal>> {
    if !access::is_valid_kind(&body.kind) {
        return Ok(Err(Refusal::Invalid("kind must be audiobook, music or podcast")));
    }
    let member = match body.member_id {
        Some(m) => m,
        None => match owner_of(pool, viewer, &body).await? {
            Some(m) => m,
            None if body.container.is_none() => {
                return Ok(Err(Refusal::Invalid("a shortcut needs a member or a container")));
            }
            None => return Ok(Err(Refusal::NotFound)),
        },
    };
    let member_id = member;
    if member_id == viewer.user_id {
        return Ok(Err(Refusal::Invalid("your own items are already in your folder")));
    }
    let in_family: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM family_members WHERE user_id = $1 AND family_id = $2)",
    )
    .bind(member_id)
    .bind(viewer.family_id)
    .fetch_one(pool)
    .await
    .context("db: check shortcut member")?;
    if !in_family {
        return Ok(Err(Refusal::NotFound));
    }

    let mut container_kind = None;
    let mut container_id = None;
    let mut release_group = None;
    let mut album_artist = None;
    let mut album = None;
    if let Some(c) = body.container {
        let trimmed = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        match (c.kind.as_str(), body.kind.as_str()) {
            ("book", access::AUDIOBOOK) | ("show", access::PODCAST) => {
                let Some(id) = c.id else {
                    return Ok(Err(Refusal::Invalid("a book or show shortcut needs its id")));
                };
                let table = if c.kind == "book" { "audiobook_books" } else { "podcast_feeds" };
                let visible: bool = sqlx::query_scalar(&format!(
                    "SELECT EXISTS (SELECT 1 FROM {table} t WHERE t.id = $5 AND t.user_id = $6 AND {})",
                    access::VISIBLE
                ))
                .bind(viewer.user_id)
                .bind(viewer.family_id)
                .bind(viewer.is_family_admin)
                .bind(body.kind.as_str())
                .bind(id)
                .bind(member_id)
                .fetch_one(pool)
                .await
                .context("db: check shortcut container")?;
                if !visible {
                    return Ok(Err(Refusal::NotFound));
                }
                container_id = Some(id);
            }
            ("album", access::MUSIC) => {
                release_group = trimmed(c.release_group);
                if release_group.is_none() {
                    album_artist = trimmed(c.album_artist);
                    album = trimmed(c.album);
                    if album.is_none() {
                        return Ok(Err(Refusal::Invalid("an album shortcut needs a release group or an album name")));
                    }
                }
                let visible: bool = sqlx::query_scalar(&format!(
                    "SELECT EXISTS (SELECT 1 FROM music_tracks t
                      WHERE t.user_id = $5 AND {}
                        AND (($6::text IS NOT NULL AND t.musicbrainz_release_group_id = $6)
                             OR ($6::text IS NULL AND lower(t.album) = lower($7)
                                 AND lower({}) = lower(COALESCE($8, 'Unknown Artist')))))",
                    access::VISIBLE,
                    crate::db::music::ALBUM_ARTIST_SQL,
                ))
                .bind(viewer.user_id)
                .bind(viewer.family_id)
                .bind(viewer.is_family_admin)
                .bind(access::MUSIC)
                .bind(member_id)
                .bind(release_group.as_deref())
                .bind(album.as_deref())
                .bind(album_artist.as_deref())
                .fetch_one(pool)
                .await
                .context("db: check album shortcut")?;
                if !visible {
                    return Ok(Err(Refusal::NotFound));
                }
            }
            ("book" | "show" | "album", _) => {
                return Ok(Err(Refusal::Invalid("the container does not match the kind")));
            }
            _ => return Ok(Err(Refusal::Invalid("container kind must be book, album or show"))),
        }
        container_kind = Some(c.kind);
    }

    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO sync_shortcuts
             (user_id, member_id, kind, container_kind, container_id, release_group, album_artist, album)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(viewer.user_id)
    .bind(member_id)
    .bind(&body.kind)
    .bind(container_kind.as_deref())
    .bind(container_id)
    .bind(release_group.as_deref())
    .bind(album_artist.as_deref())
    .bind(album.as_deref())
    .fetch_optional(pool)
    .await
    .context("db: insert shortcut")?;

    let (id, created) = match inserted {
        Some(id) => (id, true),
        None => {
            let id: Uuid = sqlx::query_scalar(
                "SELECT id FROM sync_shortcuts
                  WHERE user_id = $1 AND member_id = $2 AND kind = $3
                    AND container_kind IS NOT DISTINCT FROM $4 AND container_id IS NOT DISTINCT FROM $5
                    AND release_group IS NOT DISTINCT FROM $6
                    AND lower(album_artist) IS NOT DISTINCT FROM lower($7)
                    AND lower(album) IS NOT DISTINCT FROM lower($8)",
            )
            .bind(viewer.user_id)
            .bind(member_id)
            .bind(&body.kind)
            .bind(container_kind.as_deref())
            .bind(container_id)
            .bind(release_group.as_deref())
            .bind(album_artist.as_deref())
            .bind(album.as_deref())
            .fetch_one(pool)
            .await
            .context("db: find existing shortcut")?;
            (id, false)
        }
    };
    let shortcut = find(pool, viewer.user_id, id).await?.context("shortcut vanished after insert")?;
    Ok(Ok((shortcut, created)))
}

/// Whose book, show or album the container is, among what the caller may see and does not
/// own. `None` when nothing matches.
async fn owner_of(pool: &PgPool, viewer: Viewer, body: &NewShortcut) -> anyhow::Result<Option<Uuid>> {
    let Some(c) = &body.container else { return Ok(None) };
    let (sql, table) = match c.kind.as_str() {
        "book" => ("", "audiobook_books"),
        "show" => ("", "podcast_feeds"),
        "album" => ("album", ""),
        _ => return Ok(None),
    };
    if sql.is_empty() {
        let Some(id) = c.id else { return Ok(None) };
        return sqlx::query_scalar(&format!("SELECT user_id FROM {table} WHERE id = $1"))
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("db: container owner");
    }
    sqlx::query_scalar(&format!(
        "SELECT t.user_id FROM music_tracks t
          WHERE t.user_id <> $1 AND {}
            AND (($5::text IS NOT NULL AND t.musicbrainz_release_group_id = $5)
                 OR ($5::text IS NULL AND lower(t.album) = lower($6)
                     AND lower({}) = lower(COALESCE($7, 'Unknown Artist'))))
          ORDER BY t.created_at LIMIT 1",
        access::VISIBLE,
        crate::db::music::ALBUM_ARTIST_SQL,
    ))
    .bind(viewer.user_id)
    .bind(viewer.family_id)
    .bind(viewer.is_family_admin)
    .bind(access::MUSIC)
    .bind(c.release_group.as_deref().map(str::trim).filter(|v| !v.is_empty()))
    .bind(c.album.as_deref())
    .bind(c.album_artist.as_deref())
    .fetch_optional(pool)
    .await
    .context("db: album owner")
}

pub async fn remove(pool: &PgPool, user_id: Uuid, id: Uuid) -> anyhow::Result<bool> {
    let done = sqlx::query("DELETE FROM sync_shortcuts WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete shortcut")?;
    Ok(done.rows_affected() > 0)
}
