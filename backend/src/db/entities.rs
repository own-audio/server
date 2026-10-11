// SPDX-License-Identifier: AGPL-3.0-or-later
//! Music entities (migration 0100; docs/data-model-plan.md). They are kept in
//! step by triggers; this module clears what nothing points at any more and
//! carries the corrections (migration 0104): rename, merge and split
//! contributors, rename, merge and regroup albums. Book credits (migration
//! 0102) count like any other.
use anyhow::Context;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Removes albums, release groups, recordings, genres and contributors that no
/// track or credit refers to — left behind by a retag, a deletion or a user
/// moving household. Returns how many rows went.
pub async fn sweep_orphans(pool: &PgPool) -> anyhow::Result<u64> {
    let mut gone = 0;
    for sql in [
        // A corrected album stays, like a contributor: its keys hold the correction.
        "DELETE FROM albums a WHERE a.edited_at IS NULL AND a.release_group_set_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM music_tracks_all t WHERE t.album_id = a.id)",
        "DELETE FROM release_groups rg WHERE NOT EXISTS (SELECT 1 FROM albums a WHERE a.release_group_id = rg.id)",
        "DELETE FROM recordings r WHERE NOT EXISTS (SELECT 1 FROM music_tracks_all t WHERE t.recording_id = r.id)",
        "DELETE FROM genres g WHERE NOT EXISTS (SELECT 1 FROM track_genres tg WHERE tg.genre_id = g.id)",
        // Album credits go with their album; release-group and recording credits are not written yet.
        "DELETE FROM credits c WHERE c.item_kind = 'album' AND NOT EXISTS (SELECT 1 FROM albums a WHERE a.id = c.item_id)",
        // One made or corrected by hand stays, credited or not (an author created
        // before any book is linked to them).
        "DELETE FROM contributors k WHERE k.edited_at IS NULL AND NOT EXISTS (SELECT 1 FROM credits c WHERE c.contributor_id = k.id)",
    ] {
        gone += sqlx::query(sql).execute(pool).await.context("db: sweep orphaned music entities")?.rows_affected();
    }
    Ok(gone)
}

// ── Corrections ──────────────────────────────────────────────────────────────

pub enum Renamed {
    Done,
    NotFound,
    /// Another contributor of the household already goes by the new name: most
    /// likely the same one, to be merged rather than renamed.
    Taken(Uuid),
}

/// The name shown wherever the contributor is credited. The old spelling stays
/// a key, so the next scan of a file still spelled that way finds this
/// contributor; the new one becomes a key too.
pub async fn rename_contributor(
    tx: &mut Transaction<'_, Postgres>,
    family_id: Uuid,
    id: Uuid,
    name: &str,
) -> anyhow::Result<Renamed> {
    let name = name.trim();
    let taken: Option<Uuid> = sqlx::query_scalar(
        "SELECT contributor_id FROM contributor_keys
          WHERE family_id = $1 AND key = 'name:' || lower($2) AND contributor_id <> $3
          LIMIT 1",
    )
    .bind(family_id)
    .bind(name)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .context("db: who has the new name")?;
    if let Some(other) = taken {
        return Ok(Renamed::Taken(other));
    }
    let updated = sqlx::query(
        "UPDATE contributors SET name = $3, edited_at = now(), updated_at = now() WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(name)
    .execute(&mut **tx)
    .await
    .context("db: rename contributor")?
    .rows_affected();
    if updated == 0 {
        return Ok(Renamed::NotFound);
    }
    sqlx::query(
        "INSERT INTO contributor_keys (family_id, key, contributor_id) VALUES ($1, 'name:' || lower($2), $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(family_id)
    .bind(name)
    .bind(id)
    .execute(&mut **tx)
    .await
    .context("db: key renamed contributor")?;
    sqlx::query("SELECT music_refresh_album_lines(ARRAY[$1])")
        .bind(id)
        .execute(&mut **tx)
        .await
        .context("db: album lines after rename")?;
    Ok(Renamed::Done)
}

/// Whether every one of `ids` is a contributor of the household.
async fn household_contributors(tx: &mut Transaction<'_, Postgres>, family_id: Uuid, ids: &[Uuid]) -> anyhow::Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM contributors WHERE family_id = $1 AND id = ANY($2)")
        .bind(family_id)
        .bind(ids)
        .fetch_one(&mut **tx)
        .await
        .context("db: household contributors")?;
    Ok(n == ids.len() as i64)
}

/// `from` is the same contributor as `into` and goes, leaving `into` with
/// everything it had (contributor_merge). False when either is not the
/// household's.
pub async fn merge_contributors(pool: &PgPool, family_id: Uuid, from: Uuid, into: Uuid) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin merge contributors")?;
    if !household_contributors(&mut tx, family_id, &[from, into]).await? {
        return Ok(false);
    }
    sqlx::query("SELECT contributor_merge($1, $2)")
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await
        .context("db: merge contributors")?;
    tx.commit().await.context("db: commit merge contributors")?;
    Ok(true)
}

/// The contributor is several, named `names` in order (contributor_split);
/// returns them as (id, name). `None` when it is not the household's.
pub async fn split_contributor(
    pool: &PgPool,
    family_id: Uuid,
    id: Uuid,
    names: &[String],
) -> anyhow::Result<Option<Vec<(Uuid, String)>>> {
    let mut tx = pool.begin().await.context("db: begin split contributor")?;
    if !household_contributors(&mut tx, family_id, &[id]).await? {
        return Ok(None);
    }
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT contributor_id FROM contributor_split($1, $2)")
        .bind(id)
        .bind(names)
        .fetch_all(&mut *tx)
        .await
        .context("db: split contributor")?;
    // A statement of its own: the one calling the function cannot see the
    // contributors it made.
    let named: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM contributors WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .context("db: name split parts")?;
    let parts = ids
        .iter()
        .filter_map(|id| named.iter().find(|(n, _)| n == id).cloned())
        .collect();
    tx.commit().await.context("db: commit split contributor")?;
    Ok(Some(parts))
}

/// The album's title, everywhere it is shown. Tracks keep matching it by
/// their tags (its keys), so the next scan does not undo it.
pub async fn rename_album(pool: &PgPool, family_id: Uuid, id: Uuid, title: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query(
        "UPDATE albums SET title = $3, edited_at = now(), updated_at = now() WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(title.trim())
    .execute(pool)
    .await
    .context("db: rename album")?
    .rows_affected()
        == 1)
}

/// Puts the album in a release group of the household — another edition's,
/// to group them — or, with `None`, in one of its own. Tags stop moving it.
/// Returns the group, or `None` when the album or group is not the household's.
pub async fn set_album_release_group(
    pool: &PgPool,
    family_id: Uuid,
    id: Uuid,
    release_group: Option<Uuid>,
) -> anyhow::Result<Option<Uuid>> {
    let mut tx = pool.begin().await.context("db: begin regroup album")?;
    let group: Option<Uuid> = match release_group {
        Some(group) => sqlx::query_scalar("SELECT id FROM release_groups WHERE id = $1 AND family_id = $2")
            .bind(group)
            .bind(family_id)
            .fetch_optional(&mut *tx)
            .await
            .context("db: find release group")?,
        None => sqlx::query_scalar(
            "INSERT INTO release_groups (family_id, title, match_key)
             SELECT family_id, title, 'album:' || id FROM albums WHERE id = $1 AND family_id = $2
             ON CONFLICT (family_id, match_key) DO UPDATE SET match_key = EXCLUDED.match_key
             RETURNING id",
        )
        .bind(id)
        .bind(family_id)
        .fetch_optional(&mut *tx)
        .await
        .context("db: own release group")?,
    };
    let Some(group) = group else {
        return Ok(None);
    };
    let updated = sqlx::query(
        "UPDATE albums SET release_group_id = $3, release_group_set_at = now(), updated_at = now()
          WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(group)
    .execute(&mut *tx)
    .await
    .context("db: regroup album")?
    .rows_affected();
    if updated == 0 {
        return Ok(None);
    }
    tx.commit().await.context("db: commit regroup album")?;
    Ok(Some(group))
}

/// `from` is the same release as `into`: its tracks, keys, stars and ratings
/// move there and it goes (album_merge). False when either is not the household's.
pub async fn merge_albums(pool: &PgPool, family_id: Uuid, from: Uuid, into: Uuid) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin merge albums")?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM albums WHERE family_id = $1 AND id = ANY($2)")
        .bind(family_id)
        .bind([from, into])
        .fetch_one(&mut *tx)
        .await
        .context("db: household albums")?;
    if n != 2 {
        return Ok(false);
    }
    sqlx::query("SELECT album_merge($1, $2)")
        .bind(from)
        .bind(into)
        .execute(&mut *tx)
        .await
        .context("db: merge albums")?;
    tx.commit().await.context("db: commit merge albums")?;
    Ok(true)
}
