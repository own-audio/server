// SPDX-License-Identifier: AGPL-3.0-or-later
//! Google Books — the source behind "identify this audiobook".
//!
//! One GET against `googleapis.com/books/v1/volumes`. None of the throttling
//! the music side needs (`mirror.rs`, `cover_art.rs`'s shared semaphore)
//! applies here — Google's quota is per project per day, not per second.
//!
//! **Send a key.** The API accepts keyless requests, but bills them to a
//! per-IP anonymous quota shared with every other keyless caller on that
//! address; a bare `curl` from a home connection returned
//! `429 RESOURCE_EXHAUSTED` on the first try while this was being written.
//! `GOOGLE_CLOUD__BOOKS_API_KEY` moves that quota to our own project. Keyless
//! still works and is what a self-hoster gets by default, which is exactly why
//! [`search`] reports the failure instead of returning an empty list — "no
//! matches" and "we were turned away" must not look the same.
//!
//! **It describes the print book, not the audiobook.** There is no narrator, no
//! runtime, no chapter list and no series position in this data, which is why
//! `narrator` is the one field an apply never touches. Audible/Audnexus is the
//! source that would carry those; this module is deliberately shaped so a
//! second provider can sit beside it rather than being folded into it.

use anyhow::Context;
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "audio2/0.1 (own.audio; https://own.audio)";
const SEARCH_URL: &str = "https://www.googleapis.com/books/v1/volumes";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Google's Books backend answers `503 backendFailed` to a large share of perfectly valid
/// requests — measured at roughly half of them, on queries that had succeeded seconds earlier
/// and would succeed again seconds later. It is not the query and not the parameters: the exact
/// same string alternates between 200 and 503 within one second. Retrying is the only thing that
/// helps, so it lives here rather than being every caller's problem.
const MAX_ATTEMPTS: u32 = 4;

/// Multiplied by the attempt number. The failures come back in ~300 ms, so four tries cost
/// under two seconds in the bad case and nothing at all in the common one.
const RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// Long inputs are pointless to search on and make the O(n·m) distance in
/// [`score`] expensive; the same guard Audiobookshelf added after a ReDoS
/// report, for the same reason.
const MAX_QUERY_LEN: usize = 200;

/// Raised for the one failure mode a deployment without a key hits constantly, so callers can
/// say something truer than "the lookup failed". Google answers `429` when the anonymous per-IP
/// quota — shared with every other keyless caller on that address — is spent.
#[derive(Debug, thiserror::Error)]
#[error("google books turned the request away: no API key is set, and the shared keyless quota for this server's IP address is used up")]
pub struct QuotaExhausted;

/// Google caps `maxResults` at 40 and gets noticeably worse past the first
/// page, so there is nothing to gain by asking for more.
const MAX_RESULTS: u8 = 40;

/// One search hit, already flattened out of Google's `volumeInfo` nesting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct BookCandidate {
    pub volume_id: String,
    pub title: String,
    /// Google keeps this out of `title`; audio2 has no subtitle column, so it
    /// is shown to help tell two editions apart and then dropped.
    pub subtitle: Option<String>,
    /// Joined with ", " — audio2 stores one author string per book.
    pub author: Option<String>,
    pub publisher: Option<String>,
    pub published_year: Option<i32>,
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub page_count: Option<i32>,
    pub categories: Vec<String>,
    /// Best image Google offered, already upgraded past the 128 px thumbnail
    /// where possible. May still 404 — treat a failed load as "no cover".
    pub cover_url: Option<String>,
    /// 0–100, computed here rather than by Google, which returns relevance
    /// order but no score. See [`score`].
    pub score: u8,
}

// ── Google's wire shapes ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct VolumesResponse {
    #[serde(default)]
    items: Vec<Volume>,
}

#[derive(Deserialize)]
struct Volume {
    id: String,
    #[serde(rename = "volumeInfo")]
    volume_info: Option<VolumeInfo>,
}

#[derive(Deserialize)]
struct VolumeInfo {
    title: Option<String>,
    subtitle: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    publisher: Option<String>,
    /// "2019", "2019-04" or "2019-04-23" — only the year is ever kept.
    #[serde(rename = "publishedDate")]
    published_date: Option<String>,
    description: Option<String>,
    #[serde(rename = "industryIdentifiers", default)]
    industry_identifiers: Vec<IndustryIdentifier>,
    #[serde(rename = "pageCount")]
    page_count: Option<i32>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(rename = "imageLinks")]
    image_links: Option<ImageLinks>,
}

#[derive(Deserialize)]
struct IndustryIdentifier {
    #[serde(rename = "type")]
    kind: String,
    identifier: String,
}

#[derive(Deserialize, Default)]
struct ImageLinks {
    #[serde(rename = "smallThumbnail")]
    small_thumbnail: Option<String>,
    thumbnail: Option<String>,
    small: Option<String>,
    medium: Option<String>,
    large: Option<String>,
    #[serde(rename = "extraLarge")]
    extra_large: Option<String>,
}

// ── Public API ───────────────────────────────────────────────────────────

/// Candidates for a book. At least one of `title`/`author` must be non-empty;
/// the caller enforces that before reaching here.
///
/// Runs the field-qualified query first (`intitle:`/`inauthor:`, which is what
/// makes "Dune" find the novel rather than everything mentioning it) and falls
/// back to a plain free-text search when that returns nothing. The fallback is
/// what saves a book whose title still carries an uploader's punctuation, where
/// `intitle:` matches too strictly to hit anything.
pub async fn search(
    title: Option<&str>,
    author: Option<&str>,
    limit: u8,
    api_key: Option<&str>,
) -> anyhow::Result<Vec<BookCandidate>> {
    let title = truncate(title);
    let author = truncate(author);
    let limit = limit.clamp(1, MAX_RESULTS);

    // A failed qualified search is not fatal — the fallback below rescues it. Google answers
    // `503 backendFailed` to some entirely ordinary field-qualified queries, reproducibly and
    // not transiently: `intitle:Nemesis` fails every time while `intitle:Macbeth` succeeds, and
    // `Nemesis Jo Nesbo` as plain text succeeds too. Treating that 503 as the end of the attempt
    // made the feature look broken for a book Google knows perfectly well.
    let (mut items, qualified_error) =
        match fetch(&qualified_query(title.as_deref(), author.as_deref()), limit, api_key).await {
            Ok(items) => (items, None),
            Err(error) => (Vec::new(), Some(error)),
        };

    if items.is_empty() {
        let free_text = [title.as_deref(), author.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if !free_text.trim().is_empty() {
            match fetch(&free_text, limit, api_key).await {
                Ok(fallback) => items = fallback,
                // Both attempts failed. The first error is the one worth reporting: when it is
                // a spent quota, it carries the fix.
                Err(error) => return Err(qualified_error.unwrap_or(error)),
            }
        } else if let Some(error) = qualified_error {
            return Err(error);
        }
    }

    let mut candidates: Vec<BookCandidate> = items
        .into_iter()
        .filter_map(|item| clean_result(item, title.as_deref(), author.as_deref()))
        .collect();

    // Stable, so Google's own relevance order survives as the tiebreak between
    // candidates our scoring can't separate.
    candidates.sort_by_key(|c| std::cmp::Reverse(c.score));
    Ok(candidates)
}

/// One volume by id, fetched at apply time so the server never writes a book
/// from a client-supplied copy of a search result.
pub async fn volume(volume_id: &str, api_key: Option<&str>) -> anyhow::Result<BookCandidate> {
    if volume_id.is_empty() || !volume_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        anyhow::bail!("not a Google Books volume id: {volume_id}");
    }

    let client = client()?;
    let response = send_with_retry(|| {
        client
            .get(format!("{SEARCH_URL}/{volume_id}"))
            .query(&key_param(api_key))
    })
    .await
    .context("google books: volume request failed")?;

    let item: Volume = response
        .json()
        .await
        .context("google books: could not parse the volume")?;

    clean_result(item, None, None).context("google books: volume carried no usable metadata")
}

/// Downloads the candidate's cover, preferring the upgraded URL and falling
/// back to the one Google actually returned.
///
/// The upgrade is a guess about Google's image server, not a documented API, so
/// it is verified the only way that means anything: the bytes are fetched and
/// sniffed, and anything that isn't an image falls through to the original.
/// Returns `None` when neither yields one — a bookless cover is normal, not an
/// error.
pub async fn fetch_cover(cover_url: &str) -> Option<(Vec<u8>, super::musicbrainz::ImageFormat)> {
    let mut urls = Vec::with_capacity(2);
    if let Some(upgraded) = upgrade_cover_url(cover_url) {
        urls.push(upgraded);
    }
    urls.push(cover_url.to_string());

    for url in urls {
        let Ok(bytes) = super::musicbrainz::download_bytes(&url).await else {
            continue;
        };
        if let Some(format) = super::musicbrainz::sniff_image_format(&bytes) {
            return Some((bytes, format));
        }
        tracing::debug!(url = %url, "google books cover was not an image; trying the next form");
    }
    None
}

// ── Query building ───────────────────────────────────────────────────────

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// `intitle:` / `inauthor:`. Quoted, because an unquoted multi-word value binds
/// only its first word to the field and lets the rest float free — which is how
/// a search for an author returns books that merely mention their surname.
fn qualified_query(title: Option<&str>, author: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(title) = title {
        parts.push(format!("intitle:{}", quote(title)));
    }
    if let Some(author) = author {
        parts.push(format!("inauthor:{}", quote(author)));
    }
    parts.join(" ")
}

/// Google's query syntax has no escape for a double quote inside a quoted
/// phrase, so the only safe move is to drop them.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', " ").trim())
}

/// `[]` rather than `[("key", "")]` when unset — an empty `key` is rejected
/// outright, which is worse than not sending one.
fn key_param(api_key: Option<&str>) -> Vec<(&'static str, String)> {
    api_key
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| vec![("key", key.to_string())])
        .unwrap_or_default()
}

async fn fetch(query: &str, limit: u8, api_key: Option<&str>) -> anyhow::Result<Vec<Volume>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let client = client()?;
    let limit = limit.to_string();
    let response = send_with_retry(|| {
        client
            .get(SEARCH_URL)
            .query(&[
                ("q", query),
                ("maxResults", limit.as_str()),
                ("printType", "books"),
                ("orderBy", "relevance"),
            ])
            .query(&key_param(api_key))
    })
    .await
    .context("google books: search request failed")?;

    let body: VolumesResponse = response
        .json()
        .await
        .context("google books: could not parse the search response")?;
    Ok(body.items)
}

/// Sends a request, retrying the failures that are worth retrying.
///
/// A `5xx` from Google is nearly always transient (see [`MAX_ATTEMPTS`]) and a timeout or a
/// dropped connection is the same kind of problem. A `4xx` is not: a spent quota or a disabled
/// API answers identically however many times it is asked, so those return immediately —
/// [`QuotaExhausted`] separately from the rest, because one is fixable in the Cloud console and
/// the other is not.
async fn send_with_retry<F>(build: F) -> anyhow::Result<reqwest::Response>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 1;
    loop {
        let retryable = match build().send().await {
            Ok(response) => {
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(QuotaExhausted.into());
                }
                if !response.status().is_server_error() {
                    return Ok(response.error_for_status()?);
                }
                let status = response.status();
                tracing::debug!(attempt, %status, "google books answered a server error; retrying");
                None
            }
            Err(error) if error.is_timeout() || error.is_connect() => {
                tracing::debug!(attempt, %error, "google books request did not complete; retrying");
                Some(error)
            }
            Err(error) => return Err(error.into()),
        };

        if attempt >= MAX_ATTEMPTS {
            return match retryable {
                Some(error) => Err(error.into()),
                None => Err(anyhow::anyhow!(
                    "google books kept answering with a server error after {MAX_ATTEMPTS} attempts"
                )),
            };
        }
        tokio::time::sleep(RETRY_BACKOFF * attempt).await;
        attempt += 1;
    }
}

// ── Mapping ──────────────────────────────────────────────────────────────

fn clean_result(item: Volume, query_title: Option<&str>, query_author: Option<&str>) -> Option<BookCandidate> {
    let info = item.volume_info?;
    let title = info.title?;
    let author = (!info.authors.is_empty()).then(|| info.authors.join(", "));

    let score = score(&title, author.as_deref(), query_title, query_author);

    Some(BookCandidate {
        volume_id: item.id,
        subtitle: info.subtitle,
        author,
        publisher: info.publisher,
        published_year: info.published_date.as_deref().and_then(published_year),
        description: info.description.as_deref().and_then(clean_description),
        isbn: extract_isbn(&info.industry_identifiers),
        // Google sends 0 for a volume whose page count it does not know; a book is not
        // nought pages long, and clients render the field whenever it is present.
        page_count: info.page_count.filter(|&p| p > 0),
        categories: info.categories,
        cover_url: info.image_links.as_ref().and_then(best_cover_url),
        title,
        score,
    })
}

/// Google's `description` is a blurb written for a web page: HTML tags, HTML entities, and
/// soft hyphens left in by the publisher's typesetting. Shown raw it reads as `&quot;`, stray
/// `<br>`s and words that break in the middle of a line.
///
/// Every client would otherwise clean it separately — and four cleanups is four sets of bugs —
/// so it is done here, once, for the picker and for the text `apply` writes onto the book.
/// Nothing is truncated: how much of it to show is the caller's decision, not this module's.
fn clean_description(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut text = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            // Only a real tag is markup. Publishers write `>>quote<<` as quotation marks, and
            // treating every `<` as a tag opener swallowed the whole blurb after one of those.
            '<' => match tag_at(&chars, i) {
                Some((name, next)) => {
                    if is_block_tag(&name) {
                        text.push('\n');
                    }
                    i = next;
                }
                None => {
                    text.push('<');
                    i += 1;
                }
            },
            // Soft hyphen: invisible in print, and a word broken mid-line everywhere else.
            '\u{00ad}' => i += 1,
            ch => {
                text.push(ch);
                i += 1;
            }
        }
    }

    let text = decode_entities(&text);

    // Czech publishers feed Google a single line with ` # ` where their own page had a
    // heading ("… ve dvouhře<< # O knize Čím je determinován úspěch?"). Left alone it is one
    // 1700-character wall in every client; a break there is the paragraph they meant.
    let text = text.replace(" # ", "

");

    // Paragraphs kept, runs of blank lines and of spaces collapsed.
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_run += 1;
            continue;
        }

        if !out.is_empty() {
            out.push_str(if blank_run > 0 { "\n\n" } else { "\n" });
        }

        out.push_str(&line);
        blank_run = 0;
    }

    (!out.is_empty()).then_some(out)
}

/// The tag name at `open` and the index just past its `>`, or `None` when this `<` is ordinary
/// text — no name after it, or no `>` before the blurb ends.
fn tag_at(chars: &[char], open: usize) -> Option<(String, usize)> {
    let mut i = open + 1;
    if chars.get(i) == Some(&'/') {
        i += 1;
    }
    if !chars.get(i)?.is_ascii_alphabetic() {
        return None;
    }

    let name_start = i;
    while chars.get(i).is_some_and(|c| c.is_ascii_alphanumeric()) {
        i += 1;
    }
    let close = chars[i..].iter().position(|&c| c == '>')? + i;
    Some((chars[name_start..i].iter().collect(), close + 1))
}

/// Block tags are the ones that were a line break on the publisher's page. An inline tag —
/// `<b>` around one word — is not, and treating it as one split sentences in half.
fn is_block_tag(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "p" | "br" | "div" | "li" | "ul" | "ol" | "tr" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
    )
}

/// The handful Google actually emits. A full HTML entity table would be a dependency for a
/// blurb; anything rarer is left as written rather than guessed at.
fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&hellip;", "…")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
}

/// ISBN-13 in preference to ISBN-10, ignoring the `OTHER` identifiers Google
/// emits for volumes that have no ISBN at all.
fn extract_isbn(identifiers: &[IndustryIdentifier]) -> Option<String> {
    identifiers
        .iter()
        .find(|i| i.kind == "ISBN_13")
        .or_else(|| identifiers.iter().find(|i| i.kind == "ISBN_10"))
        .map(|i| i.identifier.clone())
}

fn published_year(published_date: &str) -> Option<i32> {
    published_date.split('-').next()?.trim().parse().ok()
}

/// The largest image Google offered, by explicit key order.
///
/// Audiobookshelf takes the last key of the object and assumes it is the
/// biggest; in practice a search response usually carries only `smallThumbnail`
/// and `thumbnail`, so that lands on a 128 px image roughly every time. Naming
/// the order makes the choice deliberate, and [`upgrade_cover_url`] deals with
/// the common case where the small ones are all there is.
fn best_cover_url(links: &ImageLinks) -> Option<String> {
    let url = links
        .extra_large
        .as_ref()
        .or(links.large.as_ref())
        .or(links.medium.as_ref())
        .or(links.small.as_ref())
        .or(links.thumbnail.as_ref())
        .or(links.small_thumbnail.as_ref())?;
    Some(https(url))
}

/// Google serves these over plain http in the API response even though the
/// same host answers https.
fn https(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

/// A bigger version of a `books.google.com/books/content` thumbnail: drop the
/// page-curl overlay and raise the zoom level.
///
/// Measured, not assumed: the same volume comes back 128×198 at `zoom=1` and
/// 575×889 at `zoom=3`, which is the difference between a cover that looks
/// broken on a book detail page and one that doesn't. Still undocumented,
/// hence [`fetch_cover`]'s fallback. `None` when the URL isn't one of Google's
/// content URLs or carries no `zoom`, in which case there is nothing to
/// upgrade.
fn upgrade_cover_url(url: &str) -> Option<String> {
    if !url.contains("books.google.com/books/content") || !url.contains("zoom=") {
        return None;
    }
    let upgraded: String = url
        .split('&')
        .filter(|param| *param != "edge=curl")
        .map(|param| if param.starts_with("zoom=") { "zoom=3" } else { param })
        .collect::<Vec<_>>()
        .join("&");
    (upgraded != url).then_some(upgraded)
}

// ── Scoring ──────────────────────────────────────────────────────────────

fn truncate(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(MAX_QUERY_LEN).collect())
}

/// 0–100, weighted toward the title.
///
/// Google returns relevance order but no score, and relevance alone puts study
/// guides and "summary of…" cash-ins above the book itself. Comparing the
/// candidate back against what was actually searched for is what pushes those
/// down. A search with no author scores on title alone rather than penalising
/// every result equally, which would just reproduce Google's order.
fn score(title: &str, author: Option<&str>, query_title: Option<&str>, query_author: Option<&str>) -> u8 {
    let title_score = query_title.map(|q| similarity(&normalize(title), &normalize(q)));
    let author_score = match (query_author, author) {
        (Some(q), Some(a)) => Some(similarity(&normalize(a), &normalize(q))),
        // Asked for an author, candidate has none: no evidence either way, but
        // not a match worth ranking above one that agrees.
        (Some(_), None) => Some(0.5),
        _ => None,
    };

    let combined = match (title_score, author_score) {
        (Some(t), Some(a)) => t * 0.7 + a * 0.3,
        (Some(t), None) => t,
        (None, Some(a)) => a,
        (None, None) => 0.0,
    };
    (combined * 100.0).round().clamp(0.0, 100.0) as u8
}

/// Lowercased, unpunctuated, bracket-free, single-spaced. Enough to stop
/// "Dune (Dune Chronicles, #1)" and "Dune" from reading as different books.
fn normalize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut depth = 0usize;
    for ch in value.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 1.0 for identical strings, 0.0 for nothing in common. Containment counts as
/// a full match in the direction that matters: a candidate titled "Dune" for a
/// search of "Dune Book One" is the book we want, and raw edit distance would
/// score that no better than an unrelated title of the same length.
fn similarity(candidate: &str, query: &str) -> f64 {
    if candidate.is_empty() || query.is_empty() {
        return 0.0;
    }
    if candidate == query {
        return 1.0;
    }
    if candidate.contains(query) || query.contains(candidate) {
        return 0.95;
    }
    let distance = levenshtein(candidate, query) as f64;
    let longest = candidate.chars().count().max(query.chars().count()) as f64;
    (1.0 - distance / longest).max(0.0)
}

/// Two-row Levenshtein — the full matrix is never needed, only the distance.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }

    let mut previous: Vec<usize> = (0..=a.len()).collect();
    let mut current = vec![0usize; a.len() + 1];

    for (j, bc) in b.iter().enumerate() {
        current[0] = j + 1;
        for (i, ac) in a.iter().enumerate() {
            let substitution = previous[i] + usize::from(ac != bc);
            current[i + 1] = substitution.min(previous[i + 1] + 1).min(current[i] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[a.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_description_strips_markup_entities_and_soft_hyphens() {
        let raw = "<p><b>Adam Grant</b> &quot;p\u{00ad}íše&quot;   o výkonu.</p><p>Druhý odstavec.</p>";
        assert_eq!(
            clean_description(raw).unwrap(),
            "Adam Grant \"píše\" o výkonu.\n\nDruhý odstavec."
        );
    }

    #[test]
    fn clean_description_breaks_the_publishers_hash_headings_into_paragraphs() {
        assert_eq!(
            clean_description("Citace<< # O knize Text.").unwrap(),
            "Citace<<

O knize Text."
        );
    }

    #[test]
    fn clean_description_keeps_angle_quotes_that_are_not_tags() {
        assert_eq!(
            clean_description(">>Tato kniha<< a text").unwrap(),
            ">>Tato kniha<< a text"
        );
    }

    #[test]
    fn clean_description_drops_a_blurb_that_was_only_markup() {
        assert_eq!(clean_description("<br><br>  "), None);
    }

    #[test]
    fn qualified_query_quotes_multi_word_values() {
        assert_eq!(
            qualified_query(Some("The Way of Kings"), Some("Brandon Sanderson")),
            "intitle:\"The Way of Kings\" inauthor:\"Brandon Sanderson\""
        );
        assert_eq!(qualified_query(Some("Dune"), None), "intitle:\"Dune\"");
        assert_eq!(qualified_query(None, None), "");
    }

    #[test]
    fn quote_drops_embedded_quotes_rather_than_breaking_the_query() {
        assert_eq!(quote("The \"Good\" Book"), "\"The  Good  Book\"");
    }

    #[test]
    fn isbn_13_wins_over_isbn_10() {
        let identifiers = vec![
            IndustryIdentifier { kind: "ISBN_10".into(), identifier: "0441013597".into() },
            IndustryIdentifier { kind: "ISBN_13".into(), identifier: "9780441013593".into() },
        ];
        assert_eq!(extract_isbn(&identifiers).as_deref(), Some("9780441013593"));
    }

    #[test]
    fn volumes_without_an_isbn_report_none() {
        let identifiers = vec![IndustryIdentifier { kind: "OTHER".into(), identifier: "OCLC:12345".into() }];
        assert_eq!(extract_isbn(&identifiers), None);
    }

    #[test]
    fn published_year_takes_the_year_from_every_date_shape_google_returns() {
        assert_eq!(published_year("2019"), Some(2019));
        assert_eq!(published_year("2019-04"), Some(2019));
        assert_eq!(published_year("2019-04-23"), Some(2019));
        assert_eq!(published_year("n.d."), None);
    }

    #[test]
    fn best_cover_prefers_the_largest_key_not_the_last_one() {
        let links = ImageLinks {
            small_thumbnail: Some("http://x/small-thumb".into()),
            thumbnail: Some("http://x/thumb".into()),
            large: Some("http://x/large".into()),
            ..Default::default()
        };
        assert_eq!(best_cover_url(&links).as_deref(), Some("https://x/large"));
    }

    #[test]
    fn cover_urls_are_forced_to_https() {
        let links = ImageLinks { thumbnail: Some("http://x/thumb".into()), ..Default::default() };
        assert_eq!(best_cover_url(&links).as_deref(), Some("https://x/thumb"));
    }

    #[test]
    fn upgrading_a_thumbnail_raises_zoom_and_drops_the_page_curl() {
        let url = "https://books.google.com/books/content?id=ABC&printsec=frontcover&img=1&zoom=1&edge=curl&source=gbs_api";
        assert_eq!(
            upgrade_cover_url(url).as_deref(),
            Some("https://books.google.com/books/content?id=ABC&printsec=frontcover&img=1&zoom=3&source=gbs_api")
        );
    }

    #[test]
    fn urls_with_nothing_to_upgrade_are_left_alone() {
        assert_eq!(upgrade_cover_url("https://example.com/cover.jpg"), None);
        assert_eq!(
            upgrade_cover_url("https://books.google.com/books/content?id=ABC&img=1&zoom=3"),
            None,
            "already at zoom=3 with no curl: rewriting would produce the same URL"
        );
    }

    #[test]
    fn normalize_strips_series_parentheticals_and_punctuation() {
        assert_eq!(normalize("Dune (Dune Chronicles, #1)"), "dune");
        assert_eq!(normalize("The Hitchhiker's Guide!"), "the hitchhiker s guide");
    }

    #[test]
    fn an_exact_match_outranks_a_summary_of_the_same_book() {
        let exact = score("Dune", Some("Frank Herbert"), Some("Dune"), Some("Frank Herbert"));
        let cash_in = score(
            "Summary of Frank Herbert's Dune",
            Some("Everest Media"),
            Some("Dune"),
            Some("Frank Herbert"),
        );
        assert_eq!(exact, 100);
        assert!(exact > cash_in, "exact {exact} should beat the summary {cash_in}");
    }

    #[test]
    fn a_search_without_an_author_scores_on_the_title_alone() {
        assert_eq!(score("Dune", Some("Frank Herbert"), Some("Dune"), None), 100);
    }

    #[test]
    fn a_candidate_missing_the_author_scores_below_one_that_agrees() {
        let agrees = score("Dune", Some("Frank Herbert"), Some("Dune"), Some("Frank Herbert"));
        let unknown = score("Dune", None, Some("Dune"), Some("Frank Herbert"));
        assert!(unknown < agrees);
    }

    #[test]
    fn an_unset_or_blank_key_is_left_off_the_query_entirely() {
        assert!(key_param(None).is_empty());
        assert!(key_param(Some("   ")).is_empty());
        assert_eq!(key_param(Some("abc")), vec![("key", "abc".to_string())]);
    }

    #[test]
    fn levenshtein_matches_known_distances() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
    }
}
