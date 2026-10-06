// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client for the self-hosted `music-metadata` service.
//!
//! This replaces the public MusicBrainz API for identification. The reason is
//! throughput, not features: musicbrainz.org rate-limits to ~1 req/s **per
//! outbound IP**, which the whole backend shares, so identification was
//! serialized behind one process-wide semaphore. A 15-track album took 15
//! seconds and one user with a large library blocked everyone else's identify
//! flow for the duration. See `docs/music-metadata-plan.md`.
//!
//! The swap is a transport change, not a model change. The service returns the
//! same [`MetadataCandidate`] and [`RecordingDetail`] shapes this module always
//! produced, and MBIDs are the same MBIDs — every `musicbrainz_recording_id`
//! already stored stays valid.
//!
//! What is deliberately *not* here: a fallback to the public API. Falling back
//! would reintroduce the 1 req/s limiter silently, under exactly the load that
//! would make it hurt most, and present as an inexplicable slowdown rather than
//! a clear failure. A missing or unreachable mirror is an error.

use anyhow::{Context, bail};
use serde::Serialize;

use super::{AlbumCandidate, MetadataCandidate, PodcastCatalogEntry, PodcastCategoryCount, RecordingDetail};
use crate::app::config::MetadataConfig;

/// Local calls are single-digit milliseconds; this only exists so a hung
/// service surfaces as an error instead of holding a request open forever.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// An album is one lookup per title on the far side; a long one needs more
/// than a single track's budget.
const ALBUM_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Serialize)]
struct AlbumIdentifyRequest<'a> {
    artist: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    album: Option<&'a str>,
    tracks: Vec<AlbumTrackQuery<'a>>,
}

#[derive(Serialize)]
pub struct AlbumTrackQuery<'a> {
    pub title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

pub struct MetadataMirror<'a> {
    config: &'a MetadataConfig,
}

#[derive(Serialize)]
struct IdentifyRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    album: Option<&'a str>,
    /// The local file's own duration. Evidence the public API had no way to
    /// use: a candidate whose stored length is within a few seconds of the
    /// real file is far more likely to be the right recording than one that
    /// merely has a similar title. Matters here because MusicBrainz holds many
    /// live and compilation versions of a popular track under the same name.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    limit: u8,
}

impl<'a> MetadataMirror<'a> {
    pub fn new(config: &'a MetadataConfig) -> Self {
        Self { config }
    }

    /// Candidate recordings for a track. At least one of title/artist/album
    /// must be set — the caller enforces that before reaching here.
    pub async fn identify(
        &self,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        duration_ms: Option<u64>,
        limit: usize,
    ) -> anyhow::Result<Vec<MetadataCandidate>> {
        let body = IdentifyRequest {
            title,
            artist,
            album,
            duration_ms,
            // The service clamps this itself; clamping here too keeps the
            // request honest rather than relying on the far side to fix it.
            limit: limit.clamp(1, 25) as u8,
        };

        let response = self
            .client()?
            .post(self.url("v1/identify"))
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .context("could not reach the music metadata service")?;

        if !response.status().is_success() {
            bail!("metadata identify failed: HTTP {}", response.status());
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's identify response")
    }

    /// Which album a group of tracks is, ranked: the release holding the most
    /// of them first, and between equals an official studio album before a
    /// live recording, a compilation or a bootleg.
    pub async fn identify_album(
        &self,
        artist: &str,
        album: Option<&str>,
        tracks: Vec<AlbumTrackQuery<'_>>,
    ) -> anyhow::Result<Vec<AlbumCandidate>> {
        let response = self
            .client()?
            .post(self.url("v1/albums/identify"))
            .timeout(ALBUM_REQUEST_TIMEOUT)
            .bearer_auth(&self.config.api_key)
            .json(&AlbumIdentifyRequest { artist, album, tracks })
            .send()
            .await
            .context("could not reach the music metadata service")?;

        if !response.status().is_success() {
            bail!("metadata album identify failed: HTTP {}", response.status());
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's album response")
    }

    /// One recording by MBID, re-fetched at apply time so the server never
    /// trusts a client-held copy of a search result.
    ///
    /// `mb_release_id` picks *which* release supplies the album name and track
    /// number when a recording appears on several — passing it through is what
    /// makes applying the candidate the user actually chose give the album they
    /// saw, rather than whichever release happens to sort first.
    pub async fn recording(
        &self,
        mb_recording_id: &str,
        mb_release_id: Option<&str>,
    ) -> anyhow::Result<RecordingDetail> {
        let mut request = self
            .client()?
            .get(self.url(&format!("v1/recordings/{mb_recording_id}")))
            .bearer_auth(&self.config.api_key);
        if let Some(release) = mb_release_id {
            request = request.query(&[("release", release)]);
        }

        let response = request
            .send()
            .await
            .context("could not reach the music metadata service")?;

        if !response.status().is_success() {
            bail!(
                "metadata recording lookup failed: HTTP {}",
                response.status()
            );
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's recording response")
    }

    /// Search the Podcast Index catalogue by title and author.
    ///
    /// Replaces a per-request call to `apollo.rss.com` that carried the user's
    /// search term off the box every time anyone typed. The catalogue is local
    /// (well, on the mirror host), so the term now reaches nobody.
    ///
    /// `language` is a subtag (`en`, not `en-US`) and is optional. `active_only`
    /// defaults to true on the far side, and should stay that way: 78% of the
    /// catalogue last published over a year ago, so an unfiltered search is
    /// mostly shows that have ended.
    pub async fn podcast_search(
        &self,
        query: &str,
        language: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<PodcastCatalogEntry>> {
        let limit = limit.clamp(1, 100).to_string();
        let mut request = self
            .client()?
            .get(self.url("v1/podcasts/search"))
            .bearer_auth(&self.config.api_key)
            .query(&[("q", query), ("limit", limit.as_str())]);

        if let Some(language) = language {
            request = request.query(&[("language", language)]);
        }

        let response = request
            .send()
            .await
            .context("could not reach the metadata service")?;

        if !response.status().is_success() {
            bail!("podcast search failed: HTTP {}", response.status());
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's podcast search response")
    }

    /// Feeds similar to one catalogue feed.
    ///
    /// **Takes no user identifier and never should.** It answers "what is like
    /// this show", not "what should this person hear" — the personal half is
    /// done by the caller, which is what keeps the mirror host from learning
    /// anything about anyone. See docs/podcast-recommendations-plan.md §0.
    ///
    /// Note the far side filters candidates to the seed's own language. That
    /// is right for the normal case and is the one thing P10 lifts on purpose.
    /// `language` is a comma-separated list of base subtags; given, it replaces
    /// the seed feed's own language as the filter.
    pub async fn podcast_similar(
        &self,
        catalog_id: i64,
        language: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<PodcastCatalogEntry>> {
        let mut request = self
            .client()?
            .get(self.url(&format!("v1/podcasts/{catalog_id}/similar")))
            .bearer_auth(&self.config.api_key)
            .query(&[("limit", limit.clamp(1, 50).to_string())]);

        if let Some(language) = language {
            request = request.query(&[("language", language)]);
        }

        let response = request
            .send()
            .await
            .context("could not reach the metadata service")?;

        if !response.status().is_success() {
            bail!("podcast similar failed: HTTP {}", response.status());
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's similar response")
    }

    /// The catalogue's category list with counts.
    pub async fn podcast_categories(&self) -> anyhow::Result<Vec<PodcastCategoryCount>> {
        let response = self
            .client()?
            .get(self.url("v1/podcasts/categories"))
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .context("could not reach the metadata service")?;

        if !response.status().is_success() {
            bail!("podcast categories failed: HTTP {}", response.status());
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's categories response")
    }

    /// Best feeds in one category — the cold-start surface, usable by someone
    /// who has listened to nothing and told us nothing.
    pub async fn podcast_browse(
        &self,
        category: &str,
        language: Option<&str>,
        limit: usize,
        offset: u32,
    ) -> anyhow::Result<Vec<PodcastCatalogEntry>> {
        let limit = limit.clamp(1, 100).to_string();
        let offset = offset.to_string();
        let mut request = self
            .client()?
            .get(self.url("v1/podcasts/browse"))
            .bearer_auth(&self.config.api_key)
            .query(&[("category", category), ("limit", limit.as_str()), ("offset", offset.as_str())]);

        if let Some(language) = language {
            request = request.query(&[("language", language)]);
        }

        let response = request
            .send()
            .await
            .context("could not reach the metadata service")?;

        if !response.status().is_success() {
            bail!("podcast browse failed: HTTP {}", response.status());
        }
        response
            .json()
            .await
            .context("failed to parse the metadata service's browse response")
    }

    /// Resolve one feed in the Podcast Index catalogue.
    ///
    /// `Ok(None)` for a feed the catalogue does not know — which is normal,
    /// not an error: the catalogue is a weekly snapshot of ~4.7M feeds and a
    /// show published last Tuesday is legitimately absent. Callers treat that
    /// the same as a failed lookup, so a subscribe never depends on it.
    ///
    /// The URL is normalized on the far side by the same function the import
    /// used, so `http://host/show/` and `https://host/show` resolve to one
    /// entry. Do not normalize here as well — two implementations of the same
    /// rule drift, and the one that matters is the one the index was built
    /// with.
    pub async fn podcast_lookup(
        &self,
        feed_url: Option<&str>,
        podcast_guid: Option<&str>,
        itunes_id: Option<i64>,
    ) -> anyhow::Result<Option<PodcastCatalogEntry>> {
        let mut request = self
            .client()?
            .get(self.url("v1/podcasts/lookup"))
            .bearer_auth(&self.config.api_key);

        if let Some(url) = feed_url {
            request = request.query(&[("url", url)]);
        }
        if let Some(guid) = podcast_guid {
            request = request.query(&[("guid", guid)]);
        }
        if let Some(id) = itunes_id {
            request = request.query(&[("itunes_id", id.to_string())]);
        }

        let response = request
            .send()
            .await
            .context("could not reach the metadata service")?;

        match response.status() {
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            // The catalogue is optional in that service too: unset, it answers
            // 503 and the music half still works. Treated as "no answer"
            // rather than an error so a deployment without a catalogue does
            // not log a failure on every single subscribe.
            reqwest::StatusCode::SERVICE_UNAVAILABLE => Ok(None),
            status if status.is_success() => response
                .json()
                .await
                .map(Some)
                .context("failed to parse the metadata service's podcast lookup response"),
            status => bail!("podcast lookup failed: HTTP {status}"),
        }
    }

    fn client(&self) -> anyhow::Result<reqwest::Client> {
        Ok(reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?)
    }

    /// Tolerates a configured base URL with or without a trailing slash —
    /// a misconfiguration that would otherwise produce a double slash and a
    /// 404 that looks like a missing endpoint rather than a typo.
    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.config.base_url.trim_end_matches('/'), path)
    }
}
