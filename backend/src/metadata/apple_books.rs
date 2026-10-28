// SPDX-License-Identifier: AGPL-3.0-or-later
//! Apple's audiobook catalogue (the public iTunes Search API) for "identify
//! this audiobook": no key, and it describes the audiobook rather than the
//! printed book — its cover is the square one an audiobook app shows.
//!
//! What it lacks: a narrator, an ISBN and a page count. The publisher is only
//! in the copyright line (`© 2013 by Blackstone Audio, Inc.`), which is not a
//! field worth parsing into one. The catalogue is the US store's.

use anyhow::{Context, bail};
use serde::Deserialize;
use std::time::Duration;

use super::books::BookSource;
use super::google_books::{BookCandidate, clean_description, published_year, score, truncate};

pub const ID_PREFIX: &str = "apple:";

const USER_AGENT: &str = concat!("own.audio-server/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESULTS: u8 = 50;
/// The artwork URLs end in their size; the image server makes any size up to
/// the original's, and an audiobook's original is large.
const COVER_SIZE: &str = "1200x1200bb";

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    results: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(default)]
    wrapper_type: Option<String>,
    collection_id: Option<i64>,
    collection_name: Option<String>,
    artist_name: Option<String>,
    description: Option<String>,
    release_date: Option<String>,
    artwork_url100: Option<String>,
    primary_genre_name: Option<String>,
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// Candidates for a book, best first by our score, Apple's order breaking ties.
/// The API has no title and author fields that work together, so both go into
/// one term; with nothing found, the title alone.
pub async fn search(title: Option<&str>, author: Option<&str>, limit: u8) -> anyhow::Result<Vec<BookCandidate>> {
    let title = truncate(title);
    let author = truncate(author);
    let limit = limit.clamp(1, MAX_RESULTS).to_string();
    let client = client()?;

    let term = [title.as_deref(), author.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
    let mut items = query(&client, &term, &limit).await?;
    if items.is_empty() && author.is_some() {
        if let Some(t) = title.as_deref() {
            items = query(&client, t, &limit).await?;
        }
    }

    let mut candidates: Vec<BookCandidate> = items
        .into_iter()
        .filter_map(|item| candidate(item, title.as_deref(), author.as_deref()))
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.score));
    Ok(candidates)
}

/// One audiobook by its id, fetched again at apply time.
pub async fn audiobook(id: &str) -> anyhow::Result<BookCandidate> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        bail!("not an Apple audiobook id: {id}");
    }
    let answer: Answer = client()?
        .get("https://itunes.apple.com/lookup")
        .query(&[("id", id), ("entity", "audiobook")])
        .send()
        .await
        .context("apple: lookup request failed")?
        .error_for_status()
        .context("apple: lookup refused")?
        .json()
        .await
        .context("apple: could not parse the lookup answer")?;
    answer
        .results
        .into_iter()
        .find_map(|item| candidate(item, None, None))
        .context("apple: no such audiobook")
}

async fn query(client: &reqwest::Client, term: &str, limit: &str) -> anyhow::Result<Vec<Item>> {
    if term.trim().is_empty() {
        return Ok(Vec::new());
    }
    let answer: Answer = client
        .get("https://itunes.apple.com/search")
        .query(&[("media", "audiobook"), ("entity", "audiobook"), ("term", term), ("limit", limit)])
        .send()
        .await
        .context("apple: search request failed")?
        .error_for_status()
        .context("apple: search refused")?
        .json()
        .await
        .context("apple: could not parse the search answer")?;
    Ok(answer.results)
}

fn candidate(item: Item, query_title: Option<&str>, query_author: Option<&str>) -> Option<BookCandidate> {
    if item.wrapper_type.as_deref().is_some_and(|w| w != "audiobook") {
        return None;
    }
    let id = item.collection_id?;
    let (title, subtitle) = split_title(&item.collection_name?);
    let author = item.artist_name.filter(|a| !a.trim().is_empty());
    Some(BookCandidate {
        volume_id: format!("{ID_PREFIX}{id}"),
        source: BookSource::Apple,
        score: score(&title, author.as_deref(), query_title, query_author),
        subtitle,
        author,
        publisher: None,
        published_year: item.release_date.as_deref().and_then(published_year),
        description: item.description.as_deref().and_then(clean_description),
        isbn: None,
        page_count: None,
        categories: item.primary_genre_name.into_iter().collect(),
        cover_url: item.artwork_url100.as_deref().map(larger_cover),
        title,
    })
}

/// `Kissinger: A Biography (Unabridged)` → `Kissinger`, `A Biography`. The
/// edition note says nothing about the book, and the part after the first
/// colon is the subtitle Google and Open Library keep apart too.
fn split_title(name: &str) -> (String, Option<String>) {
    let mut name = name.trim();
    for note in ["(Unabridged)", "(Abridged)"] {
        if let Some(rest) = name.strip_suffix(note) {
            name = rest.trim_end();
        }
    }
    match name.split_once(": ") {
        Some((title, subtitle)) if !title.trim().is_empty() && !subtitle.trim().is_empty() => {
            (title.trim().to_string(), Some(subtitle.trim().to_string()))
        }
        _ => (name.to_string(), None),
    }
}

fn larger_cover(url: &str) -> String {
    match url.rsplit_once('/') {
        Some((base, file)) if file.starts_with("100x100bb") => {
            format!("{base}/{COVER_SIZE}{}", &file["100x100bb".len()..])
        }
        _ => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_lose_the_edition_note_and_keep_a_subtitle_apart() {
        assert_eq!(split_title("Kissinger: A Biography"), ("Kissinger".into(), Some("A Biography".into())));
        assert_eq!(split_title("The Code Breaker (Unabridged)"), ("The Code Breaker".into(), None));
        assert_eq!(split_title("Dune"), ("Dune".into(), None));
        assert_eq!(split_title(": odd"), (": odd".into(), None));
    }

    #[test]
    fn the_cover_is_asked_for_large() {
        assert_eq!(
            larger_cover("https://is1-ssl.mzstatic.com/image/thumb/Music122/v4/1f/7639-square.jpg/100x100bb.jpg"),
            "https://is1-ssl.mzstatic.com/image/thumb/Music122/v4/1f/7639-square.jpg/1200x1200bb.jpg"
        );
        assert_eq!(larger_cover("https://example.com/cover.jpg"), "https://example.com/cover.jpg");
    }

    #[test]
    fn a_lookup_item_becomes_a_candidate() {
        let item: Item = serde_json::from_value(serde_json::json!({
            "wrapperType": "audiobook",
            "collectionId": 1646194052,
            "collectionName": "Kissinger: A Biography",
            "artistName": "Walter Isaacson",
            "description": "By the time <b>Henry Kissinger</b> was made secretary of state&hellip;",
            "releaseDate": "2013-07-10T07:00:00Z",
            "artworkUrl100": "https://x/a.jpg/100x100bb.jpg",
            "primaryGenreName": "Biographies & Memoirs"
        }))
        .unwrap();
        let c = candidate(item, Some("Kissinger"), Some("Walter Isaacson")).unwrap();
        assert_eq!(c.volume_id, "apple:1646194052");
        assert_eq!(c.source, BookSource::Apple);
        assert_eq!(c.title, "Kissinger");
        assert_eq!(c.published_year, Some(2013));
        assert_eq!(c.description.as_deref(), Some("By the time Henry Kissinger was made secretary of state…"));
        assert_eq!(c.cover_url.as_deref(), Some("https://x/a.jpg/1200x1200bb.jpg"));
        assert_eq!(c.categories, vec!["Biographies & Memoirs".to_string()]);
        assert_eq!(c.score, 100);
    }

    #[test]
    fn other_kinds_of_item_are_left_out() {
        let item: Item = serde_json::from_value(serde_json::json!({
            "wrapperType": "artist", "collectionId": 1, "collectionName": "x"
        }))
        .unwrap();
        assert!(candidate(item, None, None).is_none());
    }
}
