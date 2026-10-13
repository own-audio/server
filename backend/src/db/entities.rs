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
        // Where each device last was, in an item that is gone (0108: no foreign key).
        "DELETE FROM progress_devices p WHERE (p.item_kind = 'track' AND NOT EXISTS (SELECT 1 FROM music_tracks_all t WHERE t.id = p.item_id))
            OR (p.item_kind = 'book' AND NOT EXISTS (SELECT 1 FROM audiobook_books_all b WHERE b.id = p.item_id))
            OR (p.item_kind = 'episode' AND NOT EXISTS (SELECT 1 FROM podcast_episodes e WHERE e.id = p.item_id))",
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
    edit: &crate::db::edits::Edit,
) -> anyhow::Result<Renamed> {
    crate::db::edits::mark(tx, edit).await?;
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
pub async fn rename_album(pool: &PgPool, family_id: Uuid, id: Uuid, title: &str, edit: &crate::db::edits::Edit) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin rename album")?;
    crate::db::edits::mark(&mut tx, edit).await?;
    let done = sqlx::query(
        "UPDATE albums SET title = $3, edited_at = now(), updated_at = now() WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(title.trim())
    .execute(&mut *tx)
    .await
    .context("db: rename album")?
    .rows_affected()
        == 1;
    tx.commit().await.context("db: commit rename album")?;
    Ok(done)
}

/// Puts the album in a release group of the household — another edition's,
/// to group them — or, with `None`, in one of its own. Tags stop moving it.
/// Returns the group, or `None` when the album or group is not the household's.
pub async fn set_album_release_group(
    pool: &PgPool,
    family_id: Uuid,
    id: Uuid,
    release_group: Option<Uuid>,
    edit: &crate::db::edits::Edit,
) -> anyhow::Result<Option<Uuid>> {
    let mut tx = pool.begin().await.context("db: begin regroup album")?;
    crate::db::edits::mark(&mut tx, edit).await?;
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

/// One credit as someone set it by hand.
pub struct CreditInput {
    pub contributor_id: Uuid,
    pub role: String,
    pub credited_as: Option<String>,
}

/// The household's contributor of that name, made if new and kept by the
/// orphan sweep (someone chose it).
pub async fn contributor_named(pool: &PgPool, family_id: Uuid, name: &str) -> anyhow::Result<Uuid> {
    let id: Uuid = sqlx::query_scalar("SELECT music_contributor($1, $2, NULL)")
        .bind(family_id)
        .bind(name.trim())
        .fetch_one(pool)
        .await
        .context("db: contributor by name")?;
    Ok(id)
}

/// Whether every one of `ids` is a contributor of the household.
pub async fn all_in_household(pool: &PgPool, family_id: Uuid, ids: &[Uuid]) -> anyhow::Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT count(DISTINCT id) FROM contributors WHERE family_id = $1 AND id = ANY($2)")
        .bind(family_id)
        .bind(ids)
        .fetch_one(pool)
        .await
        .context("db: contributors in household")?;
    let distinct: std::collections::HashSet<&Uuid> = ids.iter().collect();
    Ok(n == distinct.len() as i64)
}

/// An item's credits set by hand (docs/api-v2-plan.md, V3c). Every role in
/// `credits` becomes exactly what is given and stays so whatever the tags say
/// (`music_role_corrected`); a role corrected before and absent now goes back
/// to the tags. Albums have no tags to go back to: their credits are replaced
/// whole.
pub async fn set_credits(pool: &PgPool, kind: &str, item: Uuid, credits: &[CreditInput]) -> anyhow::Result<()> {
    let roles: Vec<String> = {
        let mut r: Vec<String> = credits.iter().map(|c| c.role.clone()).collect();
        r.sort();
        r.dedup();
        r
    };
    let mut tx = pool.begin().await.context("db: begin set credits")?;
    if kind == "album" {
        sqlx::query("DELETE FROM credits WHERE item_kind = 'album' AND item_id = $1")
            .bind(item)
            .execute(&mut *tx)
            .await
            .context("db: clear album credits")?;
    } else {
        sqlx::query(
            "DELETE FROM credits WHERE item_kind = $1 AND item_id = $2 AND (role = ANY($3) OR source = 'user')",
        )
        .bind(kind)
        .bind(item)
        .bind(&roles)
        .execute(&mut *tx)
        .await
        .context("db: clear corrected roles")?;
    }
    let mut positions: std::collections::HashMap<&str, i16> = std::collections::HashMap::new();
    for c in credits {
        let position = positions.entry(c.role.as_str()).or_insert(0);
        sqlx::query(
            "INSERT INTO credits (contributor_id, item_kind, item_id, role, position, credited_as, source)
             VALUES ($1, $2, $3, $4, $5, $6, 'user') ON CONFLICT DO NOTHING",
        )
        .bind(c.contributor_id)
        .bind(kind)
        .bind(item)
        .bind(&c.role)
        .bind(*position)
        .bind(c.credited_as.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .execute(&mut *tx)
        .await
        .context("db: set credit")?;
        *position += 1;
    }
    // Roles given back to the tags are rewritten from them now.
    let relink = match kind {
        "track" => Some("UPDATE music_tracks_all SET artist = artist WHERE id = $1"),
        "book" => Some("UPDATE audiobook_books_all SET author = author WHERE id = $1"),
        "podcast" => Some("UPDATE podcast_feeds SET author = author WHERE id = $1"),
        _ => None,
    };
    if let Some(sql) = relink {
        sqlx::query(sql).bind(item).execute(&mut *tx).await.context("db: relink after credits")?;
    }
    sqlx::query("SELECT music_credit_joins($1, $2)")
        .bind(kind)
        .bind(item)
        .execute(&mut *tx)
        .await
        .context("db: join phrases")?;
    if kind == "album" {
        sqlx::query(
            "UPDATE albums a SET artist_display = l.line, edited_at = now(), updated_at = now()
               FROM (SELECT string_agg(k.name || c.join_phrase, '' ORDER BY c.position) AS line
                       FROM credits c JOIN contributors k ON k.id = c.contributor_id
                      WHERE c.item_kind = 'album' AND c.item_id = $1 AND c.role = 'artist') l
              WHERE a.id = $1 AND l.line IS NOT NULL",
        )
        .bind(item)
        .execute(&mut *tx)
        .await
        .context("db: album line")?;
    }
    tx.commit().await.context("db: commit set credits")?;
    Ok(())
}

/// What a correction may change on an album besides its title.
#[derive(Default)]
pub struct AlbumDetails {
    pub title: Option<String>,
    pub release_date: Option<Option<String>>,
    pub country: Option<Option<String>>,
    pub label: Option<Option<String>>,
    pub barcode: Option<Option<String>>,
}

/// Sets what is given; `Some(None)` clears. Marks the album corrected.
pub async fn update_album(pool: &PgPool, family_id: Uuid, id: Uuid, d: &AlbumDetails, edit: &crate::db::edits::Edit) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin update album")?;
    crate::db::edits::mark(&mut tx, edit).await?;
    let done = sqlx::query(
        "UPDATE albums SET
            title        = COALESCE($3, title),
            release_date = CASE WHEN $4 THEN $5 ELSE release_date END,
            country      = CASE WHEN $6 THEN $7 ELSE country END,
            label        = CASE WHEN $8 THEN $9 ELSE label END,
            barcode      = CASE WHEN $10 THEN $11 ELSE barcode END,
            edited_at = now(), updated_at = now()
          WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(d.title.as_deref().map(str::trim))
    .bind(d.release_date.is_some())
    .bind(d.release_date.clone().flatten())
    .bind(d.country.is_some())
    .bind(d.country.clone().flatten())
    .bind(d.label.is_some())
    .bind(d.label.clone().flatten())
    .bind(d.barcode.is_some())
    .bind(d.barcode.clone().flatten())
    .execute(&mut *tx)
    .await
    .context("db: update album")?
    .rows_affected()
        == 1;
    tx.commit().await.context("db: commit update album")?;
    Ok(done)
}

/// A contributor's sort name and kind, corrected by hand.
pub async fn describe_contributor(
    pool: &PgPool,
    family_id: Uuid,
    id: Uuid,
    sort_name: Option<Option<&str>>,
    kind: Option<&str>,
    edit: &crate::db::edits::Edit,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin describe contributor")?;
    crate::db::edits::mark(&mut tx, edit).await?;
    sqlx::query(
        "UPDATE contributors SET
            sort_name = CASE WHEN $3 THEN $4 ELSE sort_name END,
            kind = COALESCE($5, kind), edited_at = now(), updated_at = now()
          WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(sort_name.is_some())
    .bind(sort_name.flatten().map(str::trim).filter(|s| !s.is_empty()))
    .bind(kind)
    .execute(&mut *tx)
    .await
    .context("db: describe contributor")?;
    tx.commit().await.context("db: commit describe contributor")?;
    Ok(())
}
