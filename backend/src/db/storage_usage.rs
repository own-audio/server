// SPDX-License-Identifier: AGPL-3.0-or-later
/// Family storage accounting: what a family's library actually occupies.
///
/// Storage is counted from `media_objects` rows that are *both* under the
/// family's key prefix (`f/{family_id}/…`) *and* still referenced from a live
/// library row. A deleted item sits in the trash for 30 days with its objects
/// intact, and some objects outlive their rows until the storage sweep gets to
/// them, so a plain prefix sum would show a family content they deleted.
/// Counting referenced objects only means any drift in the lists below
/// *undercounts*, never overcounts.
///
/// The trash is free (docs/file-sync-plan.md §2 item 12): books, tracks and
/// playlists are read through their views, which hide trashed rows; a book's
/// files are counted only through a live book; a trashed episode's object has
/// moved to `trashed_audio_object_id`, which is not listed here.
///
/// ⚠ Discipline: every migration that adds a `… REFERENCES media_objects`
/// column must add it to the matching `*_REFS` list below, or those objects
/// silently stop being counted. In the hosted edition that undercharges the
/// family — customer-favorable, but still a bug.
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

const AUDIOBOOK_REFS: &[(&str, &str)] = &[
    ("audiobook_books", "cover_object_id"),
    ("audiobook_files f JOIN audiobook_books b ON b.id = f.book_id", "f.audio_object_id"),
    ("audiobook_authors", "image_object_id"),
    ("audiobook_collections", "cover_object_id"),
];

const PODCAST_REFS: &[(&str, &str)] = &[
    ("podcast_feeds", "image_object_id"),
    ("podcast_episodes", "audio_object_id"),
    ("podcast_episodes", "image_object_id"),
];

const MUSIC_REFS: &[(&str, &str)] = &[
    ("music_tracks", "audio_object_id"),
    ("music_tracks", "cover_object_id"),
    ("music_playlists", "cover_object_id"),
    ("music_artist_images", "image_object_id"),
];

/// Bookmark voice notes, the audiobook-generation pipeline's artifacts, and personal-use
/// podcast episode translations (podcast-translation-plan.md). The latter were omitted here
/// when that feature shipped — exactly the class of bug this file's own warning comment
/// describes — so translated audio was invisible to storage billing entirely until this.
const OTHER_REFS: &[(&str, &str)] = &[
    ("bookmarks", "audio_object_id"),
    ("voice_profiles", "preview_object_id"),
    ("generation_jobs", "raw_object_id"),
    ("generation_jobs", "final_object_id"),
    ("generation_jobs", "extracted_object_id"),
    ("generation_jobs", "hints_object_id"),
    ("generation_jobs", "translated_object_id"),
    ("generation_chapters", "audio_object_id"),
    ("generation_blocks", "audio_object_id"),
    ("podcast_episode_translations", "translated_text_object_id"),
    ("podcast_episode_translations", "audio_object_id"),
    ("podcast_translation_blocks", "audio_object_id"),
    ("companion_files", "object_id"),
];

/// For the instance-wide dashboard's audio-vs-other split — cuts across the
/// four kind lists above rather than reusing them, since each of those mixes
/// an audio column with cover/image columns. Not "booklets" literally (no
/// such feature exists in this schema); "other" is covers, artwork, and the
/// two text/hints objects.
const INSTANCE_AUDIO_REFS: &[(&str, &str)] = &[
    ("audiobook_files f JOIN audiobook_books b ON b.id = f.book_id", "f.audio_object_id"),
    ("podcast_episodes", "audio_object_id"),
    ("music_tracks", "audio_object_id"),
    ("bookmarks", "audio_object_id"),
    ("voice_profiles", "preview_object_id"),
    ("generation_jobs", "raw_object_id"),
    ("generation_jobs", "final_object_id"),
    ("generation_jobs", "extracted_object_id"),
    ("generation_jobs", "translated_object_id"),
    ("generation_chapters", "audio_object_id"),
    ("generation_blocks", "audio_object_id"),
    ("podcast_episode_translations", "audio_object_id"),
    ("podcast_translation_blocks", "audio_object_id"),
];

const INSTANCE_OTHER_REFS: &[(&str, &str)] = &[
    ("audiobook_books", "cover_object_id"),
    ("audiobook_authors", "image_object_id"),
    ("audiobook_collections", "cover_object_id"),
    ("podcast_feeds", "image_object_id"),
    ("podcast_episodes", "image_object_id"),
    ("music_tracks", "cover_object_id"),
    ("music_playlists", "cover_object_id"),
    ("music_artist_images", "image_object_id"),
    ("generation_jobs", "hints_object_id"),
    ("podcast_episode_translations", "translated_text_object_id"),
    ("companion_files", "object_id"),
];

#[derive(Debug, Clone, Default)]
pub struct StorageBreakdown {
    pub total_bytes: i64,
    pub audiobooks_bytes: i64,
    pub podcasts_bytes: i64,
    pub music_bytes: i64,
    pub other_bytes: i64,
    /// Referenced objects whose `size_bytes` is NULL — counted as zero bytes,
    /// so a non-zero value here means the total is an undercount.
    pub unsized_objects: i64,
}

fn refs_union(refs: &[(&str, &str)]) -> String {
    refs.iter()
        .map(|(table, col)| format!("SELECT {col} AS id FROM {table} WHERE {col} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" UNION ")
}

async fn kind_storage(
    pool: &PgPool,
    bucket: &str,
    prefix: &str,
    refs: &[(&str, &str)],
) -> anyhow::Result<(i64, i64)> {
    let query = format!(
        "SELECT COALESCE(SUM(m.size_bytes), 0)::BIGINT,
                COUNT(*) FILTER (WHERE m.size_bytes IS NULL)::BIGINT
         FROM media_objects m
         WHERE m.bucket = $1
           AND m.object_key LIKE $2
           AND m.id IN ({})",
        refs_union(refs)
    );
    sqlx::query_as::<_, (i64, i64)>(&query)
        .bind(bucket)
        .bind(prefix)
        .fetch_one(pool)
        .await
        .context("db: family storage sum")
}

/// The family's referenced storage, per media kind. An object referenced from
/// two columns of the same kind (deduplicated audio) is counted once; the
/// kinds themselves never share objects in practice, so the total is their sum.
pub async fn family_storage(
    pool: &PgPool,
    bucket: &str,
    family_id: Uuid,
) -> anyhow::Result<StorageBreakdown> {
    let prefix = format!("f/{family_id}/%");

    let (audiobooks, ab_unsized) = kind_storage(pool, bucket, &prefix, AUDIOBOOK_REFS).await?;
    let (podcasts, pc_unsized) = kind_storage(pool, bucket, &prefix, PODCAST_REFS).await?;
    let (music, mu_unsized) = kind_storage(pool, bucket, &prefix, MUSIC_REFS).await?;
    let (other, ot_unsized) = kind_storage(pool, bucket, &prefix, OTHER_REFS).await?;

    Ok(StorageBreakdown {
        total_bytes: audiobooks + podcasts + music + other,
        audiobooks_bytes: audiobooks,
        podcasts_bytes: podcasts,
        music_bytes: music,
        other_bytes: other,
        unsized_objects: ab_unsized + pc_unsized + mu_unsized + ot_unsized,
    })
}

/// Audio bytes vs. everything else (covers, artwork, text hints),
/// instance-wide — not scoped to a family. For the admin dashboard only;
/// unlike `family_storage`, this ignores the `f/{family_id}/` key prefix
/// entirely (`kind_storage` called with `"%"`, matching every object_key),
/// so it's two queries total regardless of how many families exist, not
/// four-per-family.
pub async fn instance_storage_by_type(pool: &PgPool, bucket: &str) -> anyhow::Result<(i64, i64)> {
    let (audio_bytes, _) = kind_storage(pool, bucket, "%", INSTANCE_AUDIO_REFS).await?;
    let (other_bytes, _) = kind_storage(pool, bucket, "%", INSTANCE_OTHER_REFS).await?;
    Ok((audio_bytes, other_bytes))
}
