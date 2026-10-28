// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open Library (openlibrary.org) for "identify this audiobook": open data,
//! no key, and no shared quota to run out of — the default source of the
//! open-source edition (`super::books`).
//!
//! A search answers per *work*, the book across all its editions, so a hit
//! carries every edition's publishers and ISBNs in no particular order.
//! Picking one would name an edition at random, so `publisher` and `isbn`
//! stay empty. The work's description is not in the search answer either; it
//! is fetched for the first few hits, which are the ones anyone looks at.
//!
//! An audiobook edition lists its narrator among the authors
//! (`Walter Isaacson, Malcolm Hillgartner`); there is no field that tells
//! them apart, so the author is offered as Open Library has it.

use anyhow::{Context, bail};
use futures_util::future::join_all;
use serde::Deserialize;
use std::time::Duration;

use super::books::BookSource;
use super::google_books::{BookCandidate, clean_description, score, truncate};

pub const ID_PREFIX: &str = "ol:";

/// Open Library asks callers to identify themselves; an identified client
/// gets a higher rate limit than an anonymous one.
const USER_AGENT: &str = concat!("own.audio-server/", env!("CARGO_PKG_VERSION"), " (https://own.audio)");
const SEARCH_URL: &str = "https://openlibrary.org/search.json";
const FIELDS: &str = "key,title,subtitle,author_name,first_publish_year,cover_i,number_of_pages_median,subject";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESULTS: u8 = 40;
/// Descriptions cost a request each; past the first few hits nobody reads them.
const DESCRIBED: usize = 5;
const MAX_CATEGORIES: usize = 5;

#[derive(Deserialize)]
struct SearchAnswer {
    #[serde(default)]
    docs: Vec<Doc>,
}

#[derive(Deserialize)]
struct Doc {
    /// `/works/OL45804W`.
    key: String,
    title: Option<String>,
    subtitle: Option<String>,
    #[serde(default)]
    author_name: Vec<String>,
    first_publish_year: Option<i32>,
    cover_i: Option<i64>,
    number_of_pages_median: Option<i32>,
    #[serde(default)]
    subject: Vec<String>,
}

#[derive(Deserialize)]
struct Work {
    description: Option<Description>,
}

/// Plain text on most works, `{ "type": "/type/text", "value": … }` on others.
#[derive(Deserialize)]
#[serde(untagged)]
enum Description {
    Text(String),
    Typed { value: String },
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// Candidates for a book, best first. Searches the `title` and `author`
/// fields, and the two as free text when that finds nothing — the same
/// rescue `google_books::search` does for a title with an uploader's
/// punctuation left in it.
pub async fn search(title: Option<&str>, author: Option<&str>, limit: u8) -> anyhow::Result<Vec<BookCandidate>> {
    let title = truncate(title);
    let author = truncate(author);
    let limit = limit.clamp(1, MAX_RESULTS).to_string();
    let client = client()?;

    let mut fielded: Vec<(&str, &str)> = vec![("fields", FIELDS), ("limit", &limit)];
    if let Some(t) = title.as_deref() {
        fielded.push(("title", t));
    }
    if let Some(a) = author.as_deref() {
        fielded.push(("author", a));
    }
    let mut docs = query(&client, &fielded).await?;

    if docs.is_empty() {
        let free_text = [title.as_deref(), author.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        if !free_text.trim().is_empty() {
            docs = query(&client, &[("fields", FIELDS), ("limit", &limit), ("q", &free_text)]).await?;
        }
    }

    let mut candidates: Vec<BookCandidate> = docs
        .into_iter()
        .filter_map(|doc| candidate(doc, title.as_deref(), author.as_deref()))
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.score));

    let descriptions = join_all(
        candidates
            .iter()
            .take(DESCRIBED)
            .map(|c| description(&client, c.volume_id.trim_start_matches(ID_PREFIX))),
    )
    .await;
    for (candidate, description) in candidates.iter_mut().zip(descriptions) {
        candidate.description = description;
    }
    Ok(candidates)
}

/// One work by its id (`OL45804W`), fetched again at apply time.
pub async fn work(id: &str) -> anyhow::Result<BookCandidate> {
    if !is_work_id(id) {
        bail!("not an Open Library work id: {id}");
    }
    let client = client()?;
    let key = format!("key:/works/{id}");
    let doc = query(&client, &[("fields", FIELDS), ("q", &key)])
        .await?
        .into_iter()
        .next()
        .context("open library: no such work")?;
    let mut candidate = candidate(doc, None, None).context("open library: the work has no title")?;
    candidate.description = description(&client, id).await;
    Ok(candidate)
}

async fn query(client: &reqwest::Client, params: &[(&str, &str)]) -> anyhow::Result<Vec<Doc>> {
    let answer: SearchAnswer = client
        .get(SEARCH_URL)
        .query(params)
        .send()
        .await
        .context("open library: search request failed")?
        .error_for_status()
        .context("open library: search refused")?
        .json()
        .await
        .context("open library: could not parse the search answer")?;
    Ok(answer.docs)
}

/// Best effort: a work without a description, or one that will not load, is
/// a candidate without one.
async fn description(client: &reqwest::Client, id: &str) -> Option<String> {
    if !is_work_id(id) {
        return None;
    }
    let work: Work = client
        .get(format!("https://openlibrary.org/works/{id}.json"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    let text = match work.description? {
        Description::Text(text) | Description::Typed { value: text } => text,
    };
    clean_description(&text)
}

fn candidate(doc: Doc, query_title: Option<&str>, query_author: Option<&str>) -> Option<BookCandidate> {
    let id = doc.key.strip_prefix("/works/")?;
    if !is_work_id(id) {
        return None;
    }
    let title = doc.title?;
    let author = (!doc.author_name.is_empty()).then(|| doc.author_name.join(", "));
    Some(BookCandidate {
        volume_id: format!("{ID_PREFIX}{id}"),
        source: BookSource::OpenLibrary,
        score: score(&title, author.as_deref(), query_title, query_author),
        subtitle: doc.subtitle,
        author,
        publisher: None,
        published_year: doc.first_publish_year,
        description: None,
        isbn: None,
        page_count: doc.number_of_pages_median.filter(|&p| p > 0),
        categories: doc.subject.into_iter().take(MAX_CATEGORIES).collect(),
        // `-L` is the largest size the cover service keeps; `default=false`
        // makes a missing cover a 404 rather than a blank placeholder image.
        cover_url: doc.cover_i.filter(|&c| c > 0).map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg?default=false")),
        title,
    })
}

/// `OL` + digits + `W`: the only shape that goes into a URL path.
pub(crate) fn is_work_id(id: &str) -> bool {
    id.len() > 3
        && id.starts_with("OL")
        && id.ends_with('W')
        && id[2..id.len() - 1].chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_work_ids_pass() {
        assert!(is_work_id("OL45804W"));
        assert!(!is_work_id("OL45804M"), "an edition, not a work");
        assert!(!is_work_id("OLW"));
        assert!(!is_work_id("OL1W/../x"));
        assert!(!is_work_id("../OL1W"));
    }

    #[test]
    fn a_search_hit_becomes_a_candidate_without_an_edition_s_details() {
        let doc: Doc = serde_json::from_value(serde_json::json!({
            "key": "/works/OL4288871W",
            "title": "Kissinger",
            "author_name": ["Walter Isaacson", "Malcolm Hillgartner"],
            "first_publish_year": 1992,
            "cover_i": 475264,
            "number_of_pages_median": 893,
            "subject": ["Biography", "Statesmen", "Diplomats", "Foreign relations", "History", "Politics"]
        }))
        .unwrap();
        let c = candidate(doc, Some("Kissinger"), Some("Walter Isaacson")).unwrap();
        assert_eq!(c.volume_id, "ol:OL4288871W");
        assert_eq!(c.source, BookSource::OpenLibrary);
        assert_eq!(c.author.as_deref(), Some("Walter Isaacson, Malcolm Hillgartner"));
        assert_eq!(c.published_year, Some(1992));
        assert_eq!(c.cover_url.as_deref(), Some("https://covers.openlibrary.org/b/id/475264-L.jpg?default=false"));
        assert_eq!((c.publisher, c.isbn), (None, None));
        assert_eq!(c.categories.len(), MAX_CATEGORIES);
        // The narrator among the authors costs two points; the book still ranks first.
        assert_eq!(c.score, 98);
    }

    #[test]
    fn both_description_shapes_read() {
        let plain: Work = serde_json::from_str(r#"{"description":"A fox."}"#).unwrap();
        let typed: Work = serde_json::from_str(r#"{"description":{"type":"/type/text","value":"A fox."}}"#).unwrap();
        for work in [plain, typed] {
            assert!(matches!(work.description, Some(Description::Text(ref t) | Description::Typed { value: ref t }) if t == "A fox."));
        }
    }
}
