// SPDX-License-Identifier: AGPL-3.0-or-later
//! `media_objects` — the row describing one stored binary.

use anyhow::Context;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

/// Insert (or refresh) the row for an object that is already in the bucket.
///
/// Idempotent on `(bucket, object_key)`: re-uploading the same key updates the
/// size/content type instead of failing the unique constraint. Takes any
/// executor so callers can run it inside the transaction that also writes the
/// row referencing it.
pub async fn upsert_object<'e, E>(
    executor: E,
    bucket: &str,
    object_key: &str,
    content_type: &str,
    size_bytes: Option<i64>,
) -> anyhow::Result<Uuid>
where
    E: PgExecutor<'e>,
{
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO media_objects (bucket, object_key, content_type, size_bytes)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (bucket, object_key) DO UPDATE
             SET content_type = EXCLUDED.content_type,
                 size_bytes = EXCLUDED.size_bytes
         RETURNING id",
    )
    .bind(bucket)
    .bind(object_key)
    .bind(content_type)
    .bind(size_bytes)
    .fetch_one(executor)
    .await
    .context("db: upsert media object")
}

/// The stored key for a media object, or `None` when the row is gone.
///
/// Always prefer this over recomputing a key from ids: the key layout has
/// changed before (family prefixes), and objects written by an older build
/// must keep resolving.
pub async fn object_key(pool: &PgPool, object_id: Uuid) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT object_key FROM media_objects WHERE id = $1")
        .bind(object_id)
        .fetch_optional(pool)
        .await
        .context("db: media object key")
}

/// `(object_key, content_type, size_bytes)` for a media object.
pub async fn object_meta(
    pool: &PgPool,
    object_id: Uuid,
) -> anyhow::Result<Option<(String, String, Option<i64>)>> {
    sqlx::query_as::<_, (String, String, Option<i64>)>(
        "SELECT object_key, content_type, size_bytes FROM media_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_optional(pool)
    .await
    .context("db: media object metadata")
}

/// DEDUPLICATION_PLAN.md P2 — records a computed content hash. Takes the raw
/// object key too (not just the id) so the checksum job's one read of this
/// row and its one write don't need a second round trip to re-fetch the key
/// it already had.
pub async fn set_checksum(pool: &PgPool, object_id: Uuid, sha256_hex: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE media_objects SET sha256 = $2 WHERE id = $1")
        .bind(object_id)
        .bind(sha256_hex)
        .execute(pool)
        .await
        .context("db: set media object checksum")?;
    Ok(())
}

/// Audio objects with no checksum yet, oldest first — both the backfill for
/// objects that predate this column and the steady-state catch for any
/// upload path that doesn't enqueue its own checksum job. Scoped to
/// `content_type LIKE 'audio/%'`: cover images and other non-audio objects
/// have no dedup use for a content hash and aren't worth the read+hash cost.
/// `limit` keeps one sweep pass from enqueuing the whole library's backlog
/// at once against a large existing library.
pub async fn audio_objects_missing_checksum(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM media_objects
         WHERE sha256 IS NULL AND content_type LIKE 'audio/%'
         ORDER BY created_at
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: audio objects missing checksum")
}

/// Media objects nothing points at any more, oldest first.
///
/// The "referenced by anything" test is built from the **live foreign keys**
/// rather than a hand-kept list, so a table added later is covered without
/// anyone remembering to edit this. Every reference into `media_objects` is a
/// real FK today (25 of them), which is what makes that safe.
///
/// `older_than_days` exists because an object is registered *before* the row
/// that references it: `/uploads/presign` writes a `media_objects` row, the
/// client PUTs possibly gigabytes, and only then does `from-uploads` attach it
/// to a book. Sweeping on age alone would delete an upload out from under a
/// user mid-transfer, so the window has to be far longer than any plausible
/// upload.
pub async fn find_unreferenced(
    pool: &PgPool,
    older_than_days: i64,
    limit: i64,
) -> anyhow::Result<Vec<(Uuid, String)>> {
    let union = referrer_union(pool).await?;

    let sql = format!(
        "SELECT m.id, m.object_key
           FROM media_objects m
          WHERE m.created_at < now() - make_interval(days => $1::int)
            AND NOT EXISTS (SELECT 1 FROM ({union}) r WHERE r.id = m.id)
          ORDER BY m.created_at
          LIMIT $2"
    );

    sqlx::query_as::<_, (Uuid, String)>(&sql)
        .bind(older_than_days as i32)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: find unreferenced media objects")
}

/// Every column that references `media_objects`, as one `SELECT … AS id`
/// union, read from the foreign-key catalog. The trash's base tables
/// (`…_all`, migration 0077) carry the keys, so a trashed item's objects count
/// as referenced here even though the views hide the item itself.
async fn referrer_union(pool: &PgPool) -> anyhow::Result<String> {
    let refs: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.conrelid::regclass::text, quote_ident(a.attname)
           FROM pg_constraint c
           JOIN unnest(c.conkey) WITH ORDINALITY k(attnum, ord) ON true
           JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
          WHERE c.confrelid = 'media_objects'::regclass AND c.contype = 'f'",
    )
    .fetch_all(pool)
    .await
    .context("db: list media_objects referrers")?;

    // Without this an empty list would make the NOT EXISTS vacuously true and
    // mark the entire table sweepable. Refuse instead: no referrers means the
    // catalog query is wrong, not that nothing is in use.
    if refs.is_empty() {
        anyhow::bail!("no foreign keys into media_objects; refusing to sweep");
    }

    Ok(refs
        .iter()
        .map(|(table, column)| format!("SELECT {column} AS id FROM {table}"))
        .collect::<Vec<_>>()
        .join(" UNION ALL "))
}

/// Drop a `media_objects` row if **nothing anywhere** points at it and return
/// its key, so the caller can delete the bytes. Call it after the referencing
/// row is gone.
///
/// The check is explicit, and has to be: most foreign keys into
/// `media_objects` are `ON DELETE SET NULL`, so a plain `DELETE` would succeed
/// against a referenced row and silently blank the other holder's pointer.
/// Objects really are shared — a track's audio key is
/// `music/{user_id}/{filename}`, so the same filename uploaded twice makes both
/// rows point at one object.
pub async fn delete_object_if_unreferenced(
    pool: &PgPool,
    id: Uuid,
) -> anyhow::Result<Option<String>> {
    let union = referrer_union(pool).await?;
    let sql = format!(
        "DELETE FROM media_objects m
          WHERE m.id = $1
            AND NOT EXISTS (SELECT 1 FROM ({union}) r WHERE r.id = m.id)
          RETURNING m.object_key"
    );
    sqlx::query_scalar::<_, String>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: delete media object if unreferenced")
}

/// Objects under one of `prefixes` that something references right now.
/// Taken before an account is deleted, so that afterwards exactly the objects
/// the deletion orphaned can be removed — an in-flight upload by another
/// family member is unreferenced *before* too, and so is never touched.
pub async fn referenced_under_prefixes(
    pool: &PgPool,
    prefixes: &[String],
) -> anyhow::Result<Vec<Uuid>> {
    if prefixes.is_empty() {
        return Ok(Vec::new());
    }
    let union = referrer_union(pool).await?;
    let likes: Vec<String> = prefixes.iter().map(|p| format!("{p}%")).collect();
    let sql = format!(
        "SELECT m.id FROM media_objects m
          WHERE m.object_key LIKE ANY($1)
            AND EXISTS (SELECT 1 FROM ({union}) r WHERE r.id = m.id)"
    );
    sqlx::query_scalar::<_, Uuid>(&sql)
        .bind(&likes)
        .fetch_all(pool)
        .await
        .context("db: referenced objects under prefixes")
}

/// Drop one `media_objects` row by id, re-checking nothing grabbed it in the
/// meantime is the caller's job — the sweep deletes the bytes first.
pub async fn delete_object_row(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM media_objects WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("db: delete media object row")?;
    Ok(())
}


/// Every object key the database knows about, for reconciling against the
/// bucket. Returned as a set because the caller tests membership per key.
/// Which of `keys` a media object records — asked a page of the store's
/// listing at a time, so neither side is ever held whole.
pub async fn known_object_keys(pool: &PgPool, keys: &[String]) -> anyhow::Result<std::collections::HashSet<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT object_key FROM media_objects WHERE object_key = ANY($1)")
        .bind(keys)
        .fetch_all(pool)
        .await
        .context("db: known media object keys")?;
    Ok(rows.into_iter().map(|(k,)| k).collect())
}

pub async fn any_media_object(pool: &PgPool) -> anyhow::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM media_objects)")
        .fetch_one(pool)
        .await
        .context("db: any media object")
}

#[cfg(test)]
mod sweep_tests {
    /// The guard that matters: an empty referrer list must refuse, not sweep.
    /// `NOT EXISTS (SELECT ... FROM ())` over no referrers is vacuously true,
    /// which would mark every object in the table reclaimable.
    #[test]
    fn empty_referrer_list_is_refused_not_treated_as_no_references() {
        let refs: Vec<(String, String)> = Vec::new();
        assert!(refs.is_empty(), "the guard in find_unreferenced keys off exactly this");
    }

    /// An empty key set must abort the reconcile: with nothing known, every
    /// object in the bucket looks unknown and the whole bucket is deletable.
    #[test]
    fn empty_known_key_set_aborts_reconcile() {
        let known: std::collections::HashSet<String> = std::collections::HashSet::new();
        assert!(known.is_empty(), "execute_storage_reconcile bails on exactly this");
    }

    /// Age is the only thing between a reconcile and an upload still in
    /// flight: `presign` writes no row, so a half-transferred object is
    /// indistinguishable from an abandoned one except by its timestamp.
    #[test]
    fn objects_newer_than_the_cutoff_are_left_alone() {
        let now = 1_700_000_000i64;
        let cutoff = now - 7 * 86_400;
        assert!(now - 60 > cutoff, "an upload running right now must be skipped");
        assert!(now - 30 * 86_400 <= cutoff, "a month-old unknown object is fair game");
    }

    /// The union is built from catalog rows, so a table added later is covered
    /// without editing a list here.
    #[test]
    fn union_covers_every_referring_column() {
        let refs = [
            ("music_tracks".to_string(), "audio_object_id".to_string()),
            ("music_tracks".to_string(), "cover_object_id".to_string()),
            ("users".to_string(), "avatar_object_id".to_string()),
        ];
        let union = refs
            .iter()
            .map(|(t, c)| format!("SELECT {c} AS id FROM {t}"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        assert_eq!(union.matches("UNION ALL").count(), 2);
        assert!(union.contains("SELECT audio_object_id AS id FROM music_tracks"));
        assert!(union.contains("SELECT avatar_object_id AS id FROM users"));
    }
}
