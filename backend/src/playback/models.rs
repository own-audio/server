// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PodcastProgress {
    pub user_id: Uuid,
    pub episode_id: Uuid,
    pub position_secs: f64,
    pub completed: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AudiobookProgress {
    pub user_id: Uuid,
    pub book_id: Uuid,
    pub file_id: Uuid,
    pub position_secs: f64,
    pub completed: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Bookmark {
    pub id: Uuid,
    pub user_id: Uuid,
    pub episode_id: Option<Uuid>,
    pub book_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub position_secs: f64,
    pub label: Option<String>,
    pub audio_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Bands are positional (1..6), not named after frequency — see migration
/// 0028's doc comment for why. Order matches the client's fixed
/// 60/150/400/1k/2.4k/15k Hz layout.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EqSettings {
    pub user_id: Uuid,
    pub device_kind: String,
    pub enabled: bool,
    pub preamp_db: f64,
    pub band1_db: f64,
    pub band2_db: f64,
    pub band3_db: f64,
    pub band4_db: f64,
    pub band5_db: f64,
    pub band6_db: f64,
    pub preset_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserSettings {
    pub user_id: Uuid,
    pub playback_speed: f64,
    pub skip_intro_secs: i32,
    pub skip_outro_secs: i32,
    pub ab_skip_forward_secs: i32,
    pub ab_skip_backward_secs: i32,
    pub ab_playback_speed: f64,
    pub updated_at: DateTime<Utc>,
}
