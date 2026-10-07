// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AudiobookBook {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NULL = private to `user_id`; set = shared with that family.
    pub family_id: Option<Uuid>,
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
    pub cover_object_id: Option<Uuid>,
    pub total_duration_secs: Option<i32>,
    pub source_url: Option<String>,
    /// Set once the book has been matched through the Google Books identify
    /// flow — the only way to tell "identified" from "never tried".
    pub google_books_volume_id: Option<String>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub published_year: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// `upload` or `folder` (a read-only library folder).
    #[sqlx(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AudiobookFile {
    pub id: Uuid,
    pub book_id: Uuid,
    pub position: i32,
    pub title: Option<String>,
    pub duration_secs: Option<i32>,
    pub audio_object_id: Uuid,
    pub created_at: DateTime<Utc>,
    /// From `media_objects`, not `audiobook_files` itself — populated only by the queries that
    /// join for it (`list_files`, `find_file`); other call sites leave it `None` rather than
    /// pay for the join when nobody asked for size.
    pub size_bytes: Option<i64>,
    /// From `media_objects.sha256` — the same join as `size_bytes` above. Nullable even when
    /// `audio_object_id` is set: the `media_checksum` job that fills it in runs asynchronously
    /// after upload, so a just-uploaded file has no checksum yet (mirror-plan B-2).
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AudiobookChapter {
    pub id: Uuid,
    pub book_id: Uuid,
    pub file_id: Option<Uuid>,
    pub position: i32,
    pub title: String,
    pub start_time_secs: f64,
    pub created_at: DateTime<Utc>,
}

// ── Authors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
    pub image_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BookAuthor {
    pub book_id: Uuid,
    pub author_id: Uuid,
    pub role: String,
}

// ── Series ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Series {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NULL = private to `user_id`; set = shared with that family.
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SeriesBook {
    pub series_id: Uuid,
    pub book_id: Uuid,
    pub position: f64,
}

// ── Collections ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Collection {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NULL = private to `user_id`; set = shared with that family.
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub cover_object_id: Option<Uuid>,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Favorites ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Favorite {
    pub user_id: Uuid,
    pub book_id: Uuid,
    pub created_at: DateTime<Utc>,
}

// ── Tags ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
}
