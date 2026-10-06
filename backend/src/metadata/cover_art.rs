// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cover art resolution — an ordered cascade of sources, first usable hit wins.
//!
//! Cover Art Archive covers only ~66% of MusicBrainz releases, and the third
//! that is missing skews toward exactly the non-mainstream material a personal
//! library is full of. It is also not really a service we call but a redirector
//! to archive.org, so its availability is not ours to depend on.
//!
//! Hence a cascade rather than a single source. See
//! `docs/music-metadata-plan.md` §6.
//!
//! **A provider failing is normal control flow, not an error.** Each step is
//! tried, validated all the way to "these bytes really are an image", and only
//! then accepted; anything short of that falls through to the next provider
//! instead of giving up. This is what the previous single-source path could not
//! do — a Cover Art Archive redirect landing on an archive.org error page
//! ended the attempt, and the track kept no cover at all.

use super::musicbrainz::{ImageFormat, download_bytes, sniff_image_format};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Semaphore;

const USER_AGENT: &str = "audio2/0.1 (own.audio; https://own.audio)";

/// iTunes and Deezer are unauthenticated third parties whose rate limits are
/// undocumented and whose goodwill is the only thing granting us access. One
/// permit, shared process-wide, with pacing — the same shape as the MusicBrainz
/// limiter, moved here now that MusicBrainz itself is served locally.
static THIRD_PARTY: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));

/// Roughly 20 requests/minute, the (undocumented) iTunes Search ceiling and
/// comfortably under Deezer's 50-per-5-seconds.
const THIRD_PARTY_SPACING: Duration = Duration::from_millis(3000);

/// What we know about the release whose cover we want. `mb_release_id` is only
/// useful to Cover Art Archive; the text fields are what the search-based
/// providers match on, so a query carrying neither cannot be served by them.
#[derive(Debug, Clone, Copy)]
pub struct CoverQuery<'a> {
    pub mb_release_id: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
}

/// Validated cover art: bytes that have been fetched *and* confirmed to be an
/// image, together with the provider that supplied them.
pub struct CoverArt {
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
    pub source: &'static str,
}

#[async_trait]
pub trait CoverArtProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// A candidate URL, or `None` if this provider has nothing for the query.
    /// Returning `None` is the ordinary outcome, not a failure.
    async fn find(&self, query: &CoverQuery<'_>) -> Option<String>;
}

fn client(timeout: Duration) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()?)
}

/// Holds a permit across the request *and* the pacing delay, so the next
/// caller waits out the rest of the window rather than issuing back to back.
async fn paced<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let _permit = THIRD_PARTY.acquire().await.expect("semaphore never closed");
    let out = f.await;
    tokio::time::sleep(THIRD_PARTY_SPACING).await;
    out
}

// ── Cover Art Archive ────────────────────────────────────────────────────

/// First because it is keyed by release id, so when it has art it is art for
/// *this exact release* rather than a text-search guess at one.
pub struct CoverArtArchive;

#[async_trait]
impl CoverArtProvider for CoverArtArchive {
    fn name(&self) -> &'static str {
        "cover-art-archive"
    }

    async fn find(&self, query: &CoverQuery<'_>) -> Option<String> {
        let release_id = query.mb_release_id?;
        super::musicbrainz::get_cover_art_url(release_id)
            .await
            .ok()
            .flatten()
    }
}

// ── iTunes Search ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ITunesResponse {
    results: Vec<ITunesResult>,
}

#[derive(Deserialize)]
struct ITunesResult {
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
}

/// Apple's search endpoint. No key, no account, good coverage of anything
/// commercially released.
pub struct ITunes;

impl ITunes {
    /// Apple serves artwork through a resizing proxy: the path ends in
    /// `100x100bb.jpg`, and substituting the dimensions returns that size.
    ///
    /// This is undocumented and could stop working at any time, which is why
    /// the substitution is attempted rather than relied upon — if the larger
    /// URL fails, the cascade simply moves on. 1200 is a deliberate ceiling:
    /// large enough for any client, small enough to stay a reasonable download.
    fn upscale(url: &str) -> String {
        url.replace("100x100bb", "1200x1200bb")
    }
}

#[async_trait]
impl CoverArtProvider for ITunes {
    fn name(&self) -> &'static str {
        "itunes"
    }

    async fn find(&self, query: &CoverQuery<'_>) -> Option<String> {
        let term = search_term(query)?;
        let client = client(Duration::from_secs(10)).ok()?;

        let resp = paced(async {
            client
                .get("https://itunes.apple.com/search")
                .query(&[
                    ("term", term.as_str()),
                    ("entity", "album"),
                    ("limit", "1"),
                ])
                .send()
                .await
        })
        .await
        .ok()?;

        let parsed: ITunesResponse = resp.json().await.ok()?;
        let raw = parsed.results.into_iter().next()?.artwork_url_100?;
        Some(Self::upscale(&raw))
    }
}

// ── Deezer ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DeezerResponse {
    data: Vec<DeezerAlbum>,
}

#[derive(Deserialize)]
struct DeezerAlbum {
    cover_xl: Option<String>,
}

/// Last, and unauthenticated like iTunes. `cover_xl` is 1000x1000, so no URL
/// rewriting is needed here — the API names the size it serves.
pub struct Deezer;

#[async_trait]
impl CoverArtProvider for Deezer {
    fn name(&self) -> &'static str {
        "deezer"
    }

    async fn find(&self, query: &CoverQuery<'_>) -> Option<String> {
        let term = search_term(query)?;
        let client = client(Duration::from_secs(10)).ok()?;

        let resp = paced(async {
            client
                .get("https://api.deezer.com/search/album")
                .query(&[("q", term.as_str()), ("limit", "1")])
                .send()
                .await
        })
        .await
        .ok()?;

        let parsed: DeezerResponse = resp.json().await.ok()?;
        parsed.data.into_iter().next()?.cover_xl
    }
}

// ── The cascade ──────────────────────────────────────────────────────────

/// Both search providers match on free text, and an album title alone is far
/// too ambiguous to identify a release ("Greatest Hits"). Requiring the artist
/// keeps a wrong cover from being confidently attached to the wrong record.
fn search_term(query: &CoverQuery<'_>) -> Option<String> {
    let artist = query.artist?.trim();
    let album = query.album?.trim();
    if artist.is_empty() || album.is_empty() {
        return None;
    }
    Some(format!("{artist} {album}"))
}

fn providers() -> Vec<Box<dyn CoverArtProvider>> {
    // iTunes and Deezer were removed on licensing grounds (2026-08-21), not
    // because they stopped working — they were the best-covering sources here.
    //
    // Deezer's developer terms restrict use to a non-commercial environment,
    // state images "are not allowed to be stored", and forbid reproduction
    // without written authorisation; this backend caches every cover into
    // Garage and serves it from its own endpoint, so all three applied.
    // iTunes artwork may be used only to promote items in the iTunes Store,
    // adjacent to a store link, which a personal music library is not.
    //
    // Cover Art Archive is deliberately kept despite granting no licence of its
    // own (images stay their owners' copyright, "use at your own risk"): it is
    // the archive the MusicBrainz ids we already store point at, and it is where
    // every comparable music application sources album art. That is a different
    // position from using a source whose terms name this exact use and prohibit
    // it. See docs/music-metadata-sources.md.
    vec![Box::new(CoverArtArchive)]
}

/// Walks the cascade and returns the first candidate that survives being
/// downloaded and sniffed. `None` means every provider was tried and none
/// produced a usable image.
pub async fn resolve(query: &CoverQuery<'_>) -> Option<CoverArt> {
    for provider in providers() {
        let name = provider.name();

        let Some(url) = provider.find(query).await else {
            continue;
        };

        let bytes = match download_bytes(&url).await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::debug!(provider = name, %url, %err, "cover art download failed; trying next provider");
                continue;
            }
        };

        // The bytes decide, not the Content-Type: Cover Art Archive has been
        // seen serving an nginx error page as `image/jpeg`, which is how a
        // 170-byte "500 Internal Server Error" once ended up stored as a
        // track's artwork.
        let Some(format) = sniff_image_format(&bytes) else {
            tracing::debug!(
                provider = name,
                bytes = bytes.len(),
                "cover art response was not a recognized image; trying next provider"
            );
            continue;
        };

        tracing::debug!(provider = name, bytes = bytes.len(), "cover art resolved");
        return Some(CoverArt {
            bytes,
            format,
            source: name,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscales_itunes_artwork_url() {
        let raw = "https://is1-ssl.mzstatic.com/image/thumb/Music211/v4/34/07/72/x.png/100x100bb.jpg";
        assert!(ITunes::upscale(raw).ends_with("/1200x1200bb.jpg"));
    }

    #[test]
    fn leaves_unrecognized_itunes_url_alone() {
        // Apple changing its URL shape must not produce a mangled URL — an
        // unchanged one still downloads, just at whatever size it names.
        let raw = "https://example.test/artwork/600x600.jpg";
        assert_eq!(ITunes::upscale(raw), raw);
    }

    #[test]
    fn search_term_needs_both_artist_and_album() {
        let both = CoverQuery {
            mb_release_id: None,
            artist: Some("Pixies"),
            album: Some("Doolittle"),
        };
        assert_eq!(search_term(&both).as_deref(), Some("Pixies Doolittle"));

        let album_only = CoverQuery {
            mb_release_id: None,
            artist: None,
            album: Some("Greatest Hits"),
        };
        assert!(search_term(&album_only).is_none());

        let blank = CoverQuery {
            mb_release_id: None,
            artist: Some("   "),
            album: Some("Doolittle"),
        };
        assert!(search_term(&blank).is_none());
    }
}
