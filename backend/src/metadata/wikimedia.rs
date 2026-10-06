// SPDX-License-Identifier: AGPL-3.0-or-later
//! Photos of a music artist or a book author, from Wikimedia Commons via
//! Wikidata. Two lookups ([`resolve`], [`resolve_author`]) share everything
//! past finding the Commons file name — only how that file name is found
//! differs, since an author has no MusicBrainz id to anchor on.
//!
//! **Chosen for licensing, not convenience.** Commons files are under free
//! licences (CC BY, CC BY-SA, or public domain) and permit commercial use, which
//! is the only source examined that does. The alternatives all fail for a paid
//! product that caches images and serves them from its own storage:
//!
//! - **Deezer** — terms restrict use to "a non-commercial purpose and in a
//!   non-commercial environment", state images "are not allowed to be stored",
//!   and forbid reproduction without written authorisation. All three apply to
//!   us.
//! - **iTunes Search API** — artwork may be used only to promote items in the
//!   iTunes Store, adjacent to a store link. A personal library is not that.
//! - **fanart.tv** — "do not use our API for commercial use without written
//!   consent".
//!
//! The price is **attribution**, which most Commons licences require and which
//! varies per file. That is why [`ArtistImage`] carries the author, licence and
//! source page rather than just bytes: an image whose attribution was thrown
//! away cannot legally be shown.

use anyhow::Context;
use serde::Deserialize;

/// Wikimedia asks for a descriptive agent identifying the application and a
/// contact — a generic or absent one is grounds for being blocked.
const USER_AGENT: &str = "audio2/0.1 (own.audio; https://own.audio)";

/// Requested from Commons directly, which resizes server-side. Large enough for
/// a detail header on a 2× display, and roughly an order of magnitude smaller
/// than the originals (which are frequently multi-megabyte scans).
const THUMB_WIDTH: u32 = 640;

pub struct ArtistImage {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
    pub content_type: &'static str,
    pub attribution: Attribution,
}

/// What a licence obliges us to show. Kept alongside the bytes because the two
/// are inseparable — storing the image without this would make it unusable.
pub struct Attribution {
    /// The image's author, as Commons records it. Plain text: Commons stores
    /// this as HTML, and it is stripped here so no caller has to render markup
    /// from a third party.
    pub author: Option<String>,
    /// e.g. "CC BY-SA 4.0", "Public domain".
    pub license: Option<String>,
    pub license_url: Option<String>,
    /// The file's own Commons page — where a reader can verify all of the above.
    pub source_url: String,
}

// ── Wikidata ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SparqlResponse {
    results: SparqlResults,
}

#[derive(Deserialize)]
struct SparqlResults {
    bindings: Vec<SparqlBinding>,
}

#[derive(Deserialize)]
struct SparqlBinding {
    image: Option<SparqlValue>,
}

#[derive(Deserialize)]
struct SparqlValue {
    value: String,
}

/// Finds the Commons file name for an artist.
///
/// Prefers the MusicBrainz artist id, which is an exact identity. Falls back to
/// an exact English label match **restricted to entities that carry a
/// MusicBrainz artist id at all** (`P434`) — that constraint is what keeps this
/// from behaving like a fuzzy search engine and attaching a stranger's
/// photograph to a badly-tagged artist. Confirmed against the live endpoint:
/// "Unknown Artist" matches nothing.
async fn commons_file_name(artist: &str, mb_artist_id: Option<&str>) -> anyhow::Result<Option<String>> {
    let query = match mb_artist_id {
        Some(mbid) => format!(
            r#"SELECT ?image WHERE {{ ?item wdt:P434 "{}". ?item wdt:P18 ?image. }} LIMIT 1"#,
            escape_sparql(mbid)
        ),
        None => format!(
            r#"SELECT ?image WHERE {{ ?item wdt:P434 ?mbid. ?item rdfs:label "{}"@en. ?item wdt:P18 ?image. }} LIMIT 1"#,
            escape_sparql(artist)
        ),
    };
    run_sparql(&query).await
}

/// Same idea for a person who has no MusicBrainz id to anchor on: constrained
/// to Wikidata entities marked `instance of: human` (`wdt:P31 wd:Q5`) instead,
/// which rules out the same label belonging to a band, a book, or a company.
async fn commons_file_name_for_human(name: &str) -> anyhow::Result<Option<String>> {
    let query = format!(
        r#"SELECT ?image WHERE {{ ?item wdt:P31 wd:Q5. ?item rdfs:label "{}"@en. ?item wdt:P18 ?image. }} LIMIT 1"#,
        escape_sparql(name)
    );
    run_sparql(&query).await
}

async fn run_sparql(query: &str) -> anyhow::Result<Option<String>> {
    let response = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get("https://query.wikidata.org/sparql")
        .query(&[("query", query), ("format", "json")])
        .send()
        .await
        .context("wikidata query failed")?;

    // Not "nothing found" — Wikidata throttles, and a 429 or 503 here has to
    // reach the caller as a failure so the miss is not cached.
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("wikidata answered {status}");
    }
    let body: SparqlResponse = response
        .json()
        .await
        .context("failed to parse the wikidata response")?;

    Ok(file_name_from(body))
}

/// An empty result set is the one honest "no picture" in this module: Wikidata
/// answered, and had nothing. Everything else that can go wrong above is an
/// error, so it cannot be mistaken for this.
fn file_name_from(body: SparqlResponse) -> Option<String> {
    body.results
        .bindings
        .into_iter()
        .find_map(|binding| binding.image)
        // P18 values are `…/Special:FilePath/<file name>`, percent-encoded.
        .and_then(|value| value.value.rsplit('/').next().map(str::to_string))
        .and_then(|encoded| urlencoding::decode(&encoded).ok().map(|s| s.into_owned()))
}

/// SPARQL string literals are double-quoted, so a quote or backslash in an
/// artist name would otherwise break out of the literal.
fn escape_sparql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

// ── Commons ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CommonsResponse {
    query: CommonsQuery,
}

#[derive(Deserialize)]
struct CommonsQuery {
    pages: std::collections::HashMap<String, CommonsPage>,
}

#[derive(Deserialize)]
struct CommonsPage {
    imageinfo: Option<Vec<CommonsImageInfo>>,
}

#[derive(Deserialize)]
struct CommonsImageInfo {
    thumburl: Option<String>,
    url: Option<String>,
    descriptionurl: Option<String>,
    extmetadata: Option<std::collections::HashMap<String, CommonsMetaValue>>,
}

#[derive(Deserialize)]
struct CommonsMetaValue {
    value: serde_json::Value,
}

/// Why there is no picture.
///
/// The distinction is the whole point: a `Missing` answer is cached so an
/// unknown artist stops costing a round trip on every render, and an
/// `Unavailable` one must not be, or a single Wikidata rate-limit would be
/// remembered as "this artist has no photograph" forever. That is exactly what
/// happened on canary — one throttled query and Queen had no picture again
/// until the row was deleted by hand.
#[derive(Debug, PartialEq, Eq)]
pub enum Miss {
    /// Asked, answered: no source has a usable picture for this artist.
    Missing,
    /// Could not ask. Says nothing about the artist.
    Unavailable,
}

/// Looks up an artist photo, with everything needed to display it lawfully.
pub async fn resolve(artist: &str, mb_artist_id: Option<&str>) -> Result<ArtistImage, Miss> {
    let name = artist.trim();
    if name.is_empty() {
        return Err(Miss::Missing);
    }

    let file_name = commons_file_name(name, mb_artist_id)
        .await
        .map_err(|error| unavailable("wikidata lookup", name, error))?;
    resolve_from_file_name(name, file_name).await
}

/// Same lookup for a book author. There is no MusicBrainz-artist-id
/// equivalent here — authors carry no external id this codebase records — so
/// the exact-label match is constrained to entities Wikidata marks as a human
/// (`wdt:P31 wd:Q5`) instead, which is what keeps it from attaching a band's,
/// book's or company's picture to an author who happens to share their name.
pub async fn resolve_author(name: &str) -> Result<ArtistImage, Miss> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Miss::Missing);
    }

    let file_name = commons_file_name_for_human(name)
        .await
        .map_err(|error| unavailable("wikidata lookup", name, error))?;
    resolve_from_file_name(name, file_name).await
}

async fn resolve_from_file_name(name: &str, file_name: Option<String>) -> Result<ArtistImage, Miss> {
    let file_name = file_name.ok_or(Miss::Missing)?;
    let info = commons_image_info(&file_name)
        .await
        .map_err(|error| unavailable("commons imageinfo", name, error))?
        .ok_or(Miss::Missing)?;

    // The thumbnail, not the original — Commons resizes server-side, and the
    // originals are routinely multi-megabyte scans.
    let url = info.thumburl.clone().or_else(|| info.url.clone()).ok_or(Miss::Missing)?;
    let bytes = super::musicbrainz::download_bytes(&url)
        .await
        .map_err(|error| unavailable("commons download", name, error))?;
    // Commons serving something that is not an image is a bad response, not
    // evidence about the subject.
    let format = super::musicbrainz::sniff_image_format(&bytes).ok_or_else(|| {
        tracing::warn!(subject = %name, "commons returned something that is not an image");
        Miss::Unavailable
    })?;

    let meta = info.extmetadata.unwrap_or_default();
    Ok(ArtistImage {
        bytes,
        extension: format.extension(),
        content_type: format.content_type(),
        attribution: Attribution {
            author: meta.get("Artist").and_then(plain_text),
            license: meta.get("LicenseShortName").and_then(plain_text),
            license_url: meta.get("LicenseUrl").and_then(plain_text),
            source_url: info.descriptionurl.unwrap_or_else(|| {
                format!(
                    "https://commons.wikimedia.org/wiki/File:{}",
                    urlencoding::encode(&file_name)
                )
            }),
        },
    })
}

fn unavailable(step: &str, artist: &str, error: anyhow::Error) -> Miss {
    tracing::warn!(%artist, %step, %error, "artist image lookup unavailable");
    Miss::Unavailable
}

async fn commons_image_info(file_name: &str) -> anyhow::Result<Option<CommonsImageInfo>> {
    let response = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get("https://commons.wikimedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("titles", &format!("File:{file_name}")),
            ("prop", "imageinfo"),
            ("iiprop", "url|extmetadata"),
            ("iiurlwidth", &THUMB_WIDTH.to_string()),
            ("format", "json"),
        ])
        .send()
        .await
        .context("commons imageinfo query failed")?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("commons answered {status}");
    }
    let body: CommonsResponse = response
        .json()
        .await
        .context("failed to parse the commons imageinfo response")?;

    Ok(body
        .query
        .pages
        .into_values()
        .find_map(|page| page.imageinfo.and_then(|infos| infos.into_iter().next())))
}

/// Commons returns these fields as HTML — an author is often a link, and a
/// credit can be a whole paragraph of markup. Callers display this as plain
/// text, so the tags come out here rather than every consumer having to be
/// trusted with third-party HTML.
fn plain_text(value: &CommonsMetaValue) -> Option<String> {
    let raw = value.value.as_str()?;
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let trimmed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_html_commons_wraps_an_author_in() {
        let value = CommonsMetaValue {
            value: serde_json::json!("<a href=\"https://example.com\">Koh Hasebe</a>; Elektra"),
        };
        assert_eq!(plain_text(&value).as_deref(), Some("Koh Hasebe; Elektra"));
    }

    #[test]
    fn treats_markup_only_metadata_as_absent() {
        let value = CommonsMetaValue { value: serde_json::json!("<span></span>") };
        assert_eq!(plain_text(&value), None);
    }

    #[test]
    fn reads_the_commons_file_name_out_of_a_sparql_answer() {
        let body: SparqlResponse = serde_json::from_value(serde_json::json!({
            "results": { "bindings": [ { "image": { "value":
                "http://commons.wikimedia.org/wiki/Special:FilePath/Queen%20photo%2002.jpg" } } ] }
        }))
        .unwrap();
        assert_eq!(file_name_from(body).as_deref(), Some("Queen photo 02.jpg"));
    }

    #[test]
    fn an_empty_answer_is_the_only_no_picture() {
        let body: SparqlResponse =
            serde_json::from_value(serde_json::json!({ "results": { "bindings": [] } })).unwrap();
        assert_eq!(file_name_from(body), None);
    }

    /// Cached as a miss, so it must not be reachable by an artist that simply
    /// was not looked up properly.
    #[test]
    fn an_empty_artist_name_is_a_miss_without_asking_anyone() {
        assert_eq!(tokio_test::block_on(resolve("   ", None)).err(), Some(Miss::Missing));
    }

    /// A quote in an artist name would otherwise terminate the SPARQL literal
    /// early — "Guns N' Roses" is fine, but a double quote is not.
    #[test]
    fn escapes_quotes_that_would_break_out_of_a_sparql_literal() {
        assert_eq!(escape_sparql(r#"The "Band""#), r#"The \"Band\""#);
        assert_eq!(escape_sparql(r"back\slash"), r"back\\slash");
        assert_eq!(escape_sparql("Guns N' Roses"), "Guns N' Roses");
    }
}
