// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PodcastFeed {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NULL = private to `user_id`; set = shared with that family.
    pub family_id: Option<Uuid>,
    pub feed_url: String,
    pub source_type: String,
    pub youtube_channel_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub link: Option<String>,
    pub language: Option<String>,
    pub image_object_id: Option<Uuid>,
    pub image_url: Option<String>,
    pub last_refreshed_at: Option<DateTime<Utc>>,
    pub refresh_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Podcast Index catalogue id (migration 0060). NULL when the catalogue
    /// has never been asked, or was asked and did not know this feed —
    /// `catalog_checked_at` distinguishes the two.
    #[sqlx(default)]
    pub catalog_id: Option<i64>,
    #[sqlx(default)]
    pub podcast_guid: Option<String>,
    #[sqlx(default)]
    pub itunes_id: Option<i64>,
    /// From the catalogue, or from the feed's own iTunes categories when the
    /// catalogue does not know it. Empty is normal, not an error.
    #[sqlx(default)]
    pub categories: Vec<String>,
    #[sqlx(default)]
    pub popularity_score: Option<i32>,
    /// `language` folded to its subtag — filter on this, never on `language`,
    /// which carries whatever the publisher wrote (`en`, `en-us`, `en-US`, …).
    #[sqlx(default)]
    pub language_base: Option<String>,
    #[sqlx(default)]
    pub catalog_checked_at: Option<DateTime<Utc>>,
    /// Set while the server stores new episodes of this show by itself
    /// (migration 0078): episodes published from then on.
    #[sqlx(default)]
    pub auto_store_since: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PodcastEpisode {
    pub id: Uuid,
    pub feed_id: Uuid,
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i32>,
    pub episode_number: Option<i32>,
    pub season_number: Option<i32>,
    pub audio_url: Option<String>,
    pub audio_object_id: Option<Uuid>,
    pub image_url: Option<String>,
    pub image_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    /// From `media_objects`, via a `LEFT JOIN` on `audio_object_id` (nullable — an episode
    /// with no local copy yet has nothing to join against). `#[sqlx(default)]` so a future
    /// query over this type that doesn't join still maps rather than erroring (mirror-plan
    /// B-2, same technique `AudiobookFile.size_bytes` already established).
    #[sqlx(default)]
    pub size_bytes: Option<i64>,
    /// From `media_objects.sha256` — nullable even when a local copy exists: the
    /// `media_checksum` job runs asynchronously after the file lands in storage.
    #[sqlx(default)]
    pub sha256: Option<String>,
    /// `<podcast:transcript>` URL (Podcasting 2.0 namespace), when the feed
    /// publishes one — see migration 0050 and docs/podcast-translation-plan.md.
    #[sqlx(default)]
    pub transcript_url: Option<String>,
    /// MIME type of `transcript_url` (`text/vtt`, `application/srt`, `application/json`, …).
    #[sqlx(default)]
    pub transcript_type: Option<String>,
}
