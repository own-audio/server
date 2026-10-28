// SPDX-License-Identifier: AGPL-3.0-or-later
//! Where "identify this audiobook" looks: Open Library, Apple's audiobook
//! catalogue or Google Books.
//!
//! Google Books without a key fails nearly everywhere: its keyless quota is
//! shared per IP address and was spent on the first try from a home connection
//! and from a NAS. So the open-source edition lets a family admin choose, and
//! starts on Open Library, which needs no key and turns nobody away
//! (`Hooks::book_source_choice`). The hosted edition keeps Google with the
//! server's own key.
//!
//! A candidate's `volume_id` says which source it came from (`ol:`, `apple:`,
//! or Google's bare id, as before revision 25), so `apply` asks the same
//! source again even when the family has switched in between.

use serde::{Deserialize, Serialize};

use super::google_books::BookCandidate;
use super::musicbrainz::ImageFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BookSource {
    /// openlibrary.org: open data, no key. Printed editions; no narrator.
    #[serde(rename = "openlibrary")]
    OpenLibrary,
    /// Apple's audiobook catalogue (the iTunes Search API): no key, the
    /// audiobook's own square cover. No narrator, no ISBN.
    Apple,
    /// Google Books: needs an API key in practice.
    Google,
}

impl BookSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenLibrary => "openlibrary",
            Self::Apple => "apple",
            Self::Google => "google",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openlibrary" => Some(Self::OpenLibrary),
            "apple" => Some(Self::Apple),
            "google" => Some(Self::Google),
            _ => None,
        }
    }
}

/// Candidates from `source`, best first.
pub async fn search(
    source: BookSource,
    title: Option<&str>,
    author: Option<&str>,
    limit: u8,
    google_key: Option<&str>,
) -> anyhow::Result<Vec<BookCandidate>> {
    match source {
        BookSource::OpenLibrary => super::open_library::search(title, author, limit).await,
        BookSource::Apple => super::apple_books::search(title, author, limit).await,
        BookSource::Google => super::google_books::search(title, author, limit, google_key).await,
    }
}

/// An `apply` with a `volume_id` no search gives out: the client's mistake, not a failure.
#[derive(Debug, thiserror::Error)]
#[error("not a book id from an identify search: {0}")]
pub struct NotAnId(pub String);

/// One candidate again, by the id a search gave, from the source that gave it.
pub async fn fetch(volume_id: &str, google_key: Option<&str>) -> anyhow::Result<BookCandidate> {
    if let Some(id) = volume_id.strip_prefix(super::open_library::ID_PREFIX) {
        if !super::open_library::is_work_id(id) {
            return Err(NotAnId(volume_id.to_string()).into());
        }
        super::open_library::work(id).await
    } else if let Some(id) = volume_id.strip_prefix(super::apple_books::ID_PREFIX) {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(NotAnId(volume_id.to_string()).into());
        }
        super::apple_books::audiobook(id).await
    } else {
        if volume_id.is_empty() || !volume_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(NotAnId(volume_id.to_string()).into());
        }
        super::google_books::volume(volume_id, google_key).await
    }
}

/// The candidate's cover as image bytes; `None` when there is none to have.
pub async fn fetch_cover(source: BookSource, cover_url: &str) -> Option<(Vec<u8>, ImageFormat)> {
    match source {
        BookSource::Google => super::google_books::fetch_cover(cover_url).await,
        BookSource::OpenLibrary | BookSource::Apple => {
            let bytes = super::musicbrainz::download_bytes(cover_url).await.ok()?;
            let format = super::musicbrainz::sniff_image_format(&bytes)?;
            Some((bytes, format))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ids_no_search_gives_out_are_refused_before_any_request() {
        for id in ["ol:../x", "ol:OL1M", "apple:12a", "apple:", "", "a/b", "x?y"] {
            let error = fetch(id, None).await.unwrap_err();
            assert!(error.is::<NotAnId>(), "{id}: {error}");
        }
    }

    #[test]
    fn sources_round_trip_through_their_names() {
        for source in [BookSource::OpenLibrary, BookSource::Apple, BookSource::Google] {
            assert_eq!(BookSource::parse(source.as_str()), Some(source));
            assert_eq!(serde_json::to_value(source).unwrap(), source.as_str());
        }
        assert_eq!(BookSource::parse("audible"), None);
    }
}
