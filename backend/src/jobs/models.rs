// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub job_type: String,
    pub status: String,
    pub payload: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobType {
    FeedRefresh,
    BookImport,
    ThumbnailFetch,
    EpisodeDownload,
    Cleanup,
}

impl JobType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FeedRefresh => "feed_refresh",
            Self::BookImport => "book_import",
            Self::ThumbnailFetch => "thumbnail_fetch",
            Self::EpisodeDownload => "episode_download",
            Self::Cleanup => "cleanup",
        }
    }
}
