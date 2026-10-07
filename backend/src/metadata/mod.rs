// SPDX-License-Identifier: AGPL-3.0-or-later
//! Metadata module — external music metadata lookups via MusicBrainz + Cover
//! Art Archive, used by `music`'s "identify this track" flow.
//!
//! MusicBrainz previously had a client here (`47e4d83`) that was deleted as
//! collateral of an unrelated sharing-model rewrite (`6201e7c`) — this is a
//! from-scratch rebuild scoped to what the Music Implementation Plan needs
//! (search + apply for a single track), not a restore of the old surface
//! (artist/release browsing was dropped as unused).
pub mod cover_art;
pub mod google_books;
pub mod itunes;
pub mod wikimedia;
pub mod mirror;
pub mod musicbrainz;
pub mod public;
pub mod watermark;

/// Where "identify" asks: the private metadata service when it is set (the
/// hosted edition), else the public MusicBrainz API unless switched off.
pub enum Identify<'a> {
    Mirror(mirror::MetadataMirror<'a>),
    Public(public::MusicBrainz),
}

impl<'a> Identify<'a> {
    pub fn from_config(config: &'a crate::app::AppConfig) -> Option<anyhow::Result<Self>> {
        if let Some(metadata) = config.metadata.as_ref() {
            return Some(Ok(Self::Mirror(mirror::MetadataMirror::new(metadata))));
        }
        config.musicbrainz.enabled.then(|| public::MusicBrainz::new(&config.musicbrainz).map(Self::Public))
    }

    pub async fn identify(
        &self,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        duration_ms: Option<u64>,
        limit: usize,
    ) -> anyhow::Result<Vec<MetadataCandidate>> {
        match self {
            Self::Mirror(m) => m.identify(title, artist, album, duration_ms, limit).await,
            Self::Public(p) => p.identify(title, artist, album, duration_ms, limit).await,
        }
    }

    pub async fn identify_album(
        &self,
        artist: &str,
        album: Option<&str>,
        tracks: Vec<mirror::AlbumTrackQuery<'_>>,
    ) -> anyhow::Result<Vec<AlbumCandidate>> {
        match self {
            Self::Mirror(m) => m.identify_album(artist, album, tracks).await,
            Self::Public(p) => p.identify_album(artist, album, tracks).await,
        }
    }

    pub async fn recording(&self, mb_recording_id: &str, mb_release_id: Option<&str>) -> anyhow::Result<RecordingDetail> {
        match self {
            Self::Mirror(m) => m.recording(mb_recording_id, mb_release_id).await,
            Self::Public(p) => p.recording(mb_recording_id, mb_release_id).await,
        }
    }
}

use serde::{Deserialize, Serialize};

/// One feed from the Podcast Index catalogue, as `music-metadata` returns it.
///
/// **Mirrors `PodcastFeedSummary` in the music-metadata repo, and nothing
/// enforces that.** The two crates build independently, so a field renamed
/// there and not here fails at runtime as a deserialization error. Every field
/// is optional or defaulted for exactly that reason: this type is only ever
/// used to enrich a feed that already exists, so tolerating a partial answer
/// is better than failing a subscribe over a field we did not need.
#[derive(Debug, Clone, Deserialize)]
pub struct PodcastCatalogEntry {
    pub id: i64,
    /// The feed URL. Required, not optional: this is what a client passes back
    /// to `/podcasts/subscribe`, so a search result without it is useless.
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub language_base: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub popularity_score: i32,
    #[serde(default)]
    pub episode_count: i32,
    #[serde(default)]
    pub podcast_guid: Option<String>,
    #[serde(default)]
    pub itunes_id: Option<i64>,
}

/// One catalogue category and how many feeds sit in it.
#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct PodcastCategoryCount {
    pub category: String,
    pub feed_count: i64,
    /// Feeds in this category that published within the last year. This is the
    /// number worth showing a user — `feed_count` is mostly abandoned shows.
    pub active_count: i64,
}

/// One MusicBrainz recording candidate, returned from a search.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MetadataCandidate {
    pub mb_recording_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub mb_artist_id: Option<String>,
    pub album: Option<String>,
    pub mb_release_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub year: Option<i32>,
    /// Not verified with a HEAD request — a speculative Cover Art Archive
    /// URL built from `mb_release_id`. May 404; the client should treat it
    /// as best-effort. [`musicbrainz::get_cover_art_url`] does the real
    /// check, used only for the single release chosen at apply time.
    pub cover_art_url: Option<String>,
    pub score: u8,
}

/// One album a group of tracks could be, from the metadata service's
/// `v1/albums/identify`. Mirrors its `AlbumCandidate` field for field — the
/// same unenforced contract as [`MetadataCandidate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumCandidate {
    pub mb_release_id: String,
    pub mb_release_group_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub status: Option<String>,
    pub track_count: i32,
    pub matched: u32,
    pub editions: u32,
    pub cover_art_url: Option<String>,
    pub tracks: Vec<AlbumTrackMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumTrackMatch {
    /// Position in the request's track list.
    pub index: u32,
    pub mb_recording_id: String,
    pub title: String,
    pub disc: i32,
    pub position: i32,
    pub duration_ms: Option<u64>,
}

/// The full detail for one recording, fetched by id at apply time so the
/// server never trusts a client-supplied copy of search-result fields.
#[derive(Debug, Clone, Deserialize)]
pub struct RecordingDetail {
    pub mb_recording_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub mb_artist_id: Option<String>,
    pub album: Option<String>,
    pub mb_release_id: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    /// The chosen release's artist credit — "George Ezra" for a track the recording credits to
    /// "George Ezra feat. First Aid Kit". `None` from a metadata service older than 2026-09-24,
    /// hence `default`: the apply then leaves the album artist alone.
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub mb_release_group_id: Option<String>,
}
