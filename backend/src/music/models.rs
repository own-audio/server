// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MusicTrack {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NULL = private to `user_id`; set = shared with that family.
    pub family_id: Option<Uuid>,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub duration_secs: Option<i32>,
    pub cover_object_id: Option<Uuid>,
    pub audio_object_id: Uuid,
    /// Set once a track has been matched via the metadata search+apply flow.
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub musicbrainz_artist_id: Option<String>,
    /// Explicit album artist only — a file tag, a compilation, a MusicBrainz release or a manual
    /// edit. `None` means derived; use [`MusicTrack::effective_album_artist`], never this field,
    /// to decide which album a track is on.
    pub album_artist: Option<String>,
    pub is_compilation: bool,
    pub musicbrainz_release_group_id: Option<String>,
    /// `NULL` = never parsed yet; `Some("")` = parsed, the file had no
    /// embedded lyrics tag; `Some(text)` = the parsed lyrics. See
    /// `db::music::update_track_lyrics`'s own doc comment.
    pub lyrics: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Which disc of the album, and of how many; from the file's tags.
    #[sqlx(default)]
    pub disc_number: Option<i32>,
    #[sqlx(default)]
    pub disc_total: Option<i32>,
    /// From `media_objects`, joined only by `list_tracks`/`find_track` (mirror-plan B-2).
    /// `#[sqlx(default)]` so every other `TRACK_COLS`-based query — playlists, genre listing,
    /// Subsonic — keeps working unjoined, with these correctly `None` rather than an error.
    #[sqlx(default)]
    pub size_bytes: Option<i64>,
    #[sqlx(default)]
    pub sha256: Option<String>,
}

impl MusicTrack {
    /// Which artist this track's album is filed under — the Rust twin of
    /// `db::music::ALBUM_ARTIST_SQL`; the two must agree or a client's album screen would list
    /// different tracks than the album list counted.
    pub fn effective_album_artist(&self) -> Option<String> {
        self.album_artist
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                self.artist
                    .as_deref()
                    .map(primary_artist)
                    .filter(|v| !v.is_empty())
                    .map(ToOwned::to_owned)
            })
    }
}

/// The track artist without its guests: "George Ezra feat. First Aid Kit" → "George Ezra".
///
/// Only `feat.`, `ft.` and `featuring` are cut, with or without a bracket before them. `&` and
/// `and` are not — "Simon & Garfunkel" is one act, and nothing in the string tells a duo from a
/// guest. Keep in step with the pattern in `db::music::ALBUM_ARTIST_SQL`.
pub fn primary_artist(artist: &str) -> &str {
    static GUESTS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\s+[(\[]?(feat\.?|ft\.?|featuring)\s.*$").expect("valid regex")
    });
    let artist = artist.trim();
    match GUESTS.find(artist) {
        Some(m) => artist[..m.start()].trim_end(),
        None => artist,
    }
}

#[cfg(test)]
mod tests {
    use super::primary_artist;

    #[test]
    fn guests_are_cut() {
        assert_eq!(primary_artist("George Ezra feat. First Aid Kit"), "George Ezra");
        assert_eq!(primary_artist("George Ezra (feat. First Aid Kit)"), "George Ezra");
        assert_eq!(primary_artist("Calvin Harris ft. Rihanna"), "Calvin Harris");
        assert_eq!(primary_artist("Santana featuring Rob Thomas"), "Santana");
        assert_eq!(primary_artist("Queen FEAT. David Bowie"), "Queen");
    }

    #[test]
    fn duos_and_lookalikes_are_kept() {
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon & Garfunkel");
        assert_eq!(primary_artist("Earth, Wind and Fire"), "Earth, Wind and Fire");
        assert_eq!(primary_artist("Daft Punk"), "Daft Punk");
        assert_eq!(primary_artist("Featurecast"), "Featurecast");
        assert_eq!(primary_artist("  Queen  "), "Queen");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MusicPlaylist {
    pub id: Uuid,
    pub user_id: Uuid,
    /// NULL = private to `user_id`; set = shared with that family.
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub cover_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set when the smart-playlist generator made it; otherwise an ordinary playlist.
    pub generated_at: Option<DateTime<Utc>>,
    /// Set when the listener chose to keep a generated playlist.
    pub kept_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MusicPlaylistTrack {
    pub id: Uuid,
    pub playlist_id: Uuid,
    pub track_id: Uuid,
    pub position: i32,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MusicProgress {
    pub user_id: Uuid,
    pub track_id: Uuid,
    pub position_secs: f64,
    pub completed: bool,
    pub updated_at: DateTime<Utc>,
}
