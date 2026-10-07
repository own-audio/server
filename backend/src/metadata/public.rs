// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identify through the public MusicBrainz web service, for a server without
//! the private metadata service (`super::mirror`).
//!
//! The same three answers the mirror gives — recording candidates, which
//! album a group of tracks is, one recording's detail — built from
//! `musicbrainz.org/ws/2`. MusicBrainz allows one request a second per
//! client and asks for a `User-Agent` that says who is calling
//! (https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting), so every call
//! here waits its turn. That makes it fit for a person pressing "identify",
//! not for a background sweep over a library: the release backfill stays
//! mirror-only.
//!
//! The album matching (`normalize`, `title_score`, `match_tracks`, the
//! ranking) is the mirror service's, so both editions pick the same album
//! from the same tracks.

use super::{AlbumCandidate, AlbumTrackMatch, MetadataCandidate, RecordingDetail, mirror::AlbumTrackQuery};
use crate::app::config::MusicBrainzConfig;
use anyhow::{Context, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

const BASE: &str = "https://musicbrainz.org/ws/2";
/// A little over a second: MusicBrainz counts per second, and a request that
/// lands on the boundary is the one that gets a 503.
const SPACING: Duration = Duration::from_millis(1100);
/// Releases looked up in full for an album match. Each is a request, so a
/// second each; the search before them already ranked them by hits.
const ALBUM_LOOKUPS: usize = 5;
/// Track titles searched to find the releases that hold them.
const ALBUM_TITLE_SEARCHES: usize = 3;
const MAX_ALBUM_CANDIDATES: usize = 8;
const DURATION_TOLERANCE_MS: i64 = 15_000;
const PREFIX_DURATION_MS: i64 = 10_000;

/// The time of the last request, process-wide: the limit is per client.
static LAST: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);

pub struct MusicBrainz {
    http: reqwest::Client,
}

impl MusicBrainz {
    pub fn new(config: &MusicBrainzConfig) -> anyhow::Result<Self> {
        let contact = config.contact.as_deref().filter(|c| !c.trim().is_empty()).unwrap_or("https://github.com/own-audio/server");
        let http = reqwest::Client::builder()
            .user_agent(format!("own.audio-server/{} ( {contact} )", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { http })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<T> {
        for attempt in 0..2 {
            {
                let mut last = LAST.lock().await;
                if let Some(at) = *last {
                    let wait = SPACING.saturating_sub(at.elapsed());
                    tokio::time::sleep(wait).await;
                }
                *last = Some(Instant::now());
            }
            let response = self
                .http
                .get(format!("{BASE}/{path}"))
                .query(query)
                .query(&[("fmt", "json")])
                .send()
                .await
                .context("could not reach musicbrainz.org")?;
            // 503 is MusicBrainz saying "slow down"; one more try after a pause.
            if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE && attempt == 0 {
                tokio::time::sleep(SPACING).await;
                continue;
            }
            if !response.status().is_success() {
                bail!("musicbrainz.org answered HTTP {}", response.status());
            }
            return response.json().await.context("unexpected answer from musicbrainz.org");
        }
        bail!("musicbrainz.org is rate limiting this server; try again in a moment")
    }

    /// Candidate recordings for a track, best first.
    pub async fn identify(
        &self,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        duration_ms: Option<u64>,
        limit: usize,
    ) -> anyhow::Result<Vec<MetadataCandidate>> {
        let query = lucene(&[("recording", title), ("artist", artist), ("release", album)]);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 25).to_string();
        let found: RecordingSearch = self.get("recording", &[("query", &query), ("limit", &limit)]).await?;
        // Many versions of a song score 100 on its title alone; between equals
        // the one on an official studio album, earliest, is the one meant.
        let mut ranked: Vec<(MetadataCandidate, (u8, i32))> = found
            .recordings
            .into_iter()
            .map(|r| {
                let kind = preferred_release(&r.releases, album).map_or((u8::MAX, i32::MAX), |rel| {
                    (release_penalty(rel), year(rel.date.as_deref()).unwrap_or(i32::MAX))
                });
                (r.into_candidate(album, duration_ms), kind)
            })
            .collect();
        ranked.sort_by(|(a, ak), (b, bk)| b.score.cmp(&a.score).then(ak.cmp(bk)));
        Ok(ranked.into_iter().map(|(c, _)| c).collect())
    }

    /// Which album a group of tracks is, ranked as the metadata service ranks.
    pub async fn identify_album(
        &self,
        artist: &str,
        album: Option<&str>,
        tracks: Vec<AlbumTrackQuery<'_>>,
    ) -> anyhow::Result<Vec<AlbumCandidate>> {
        // Releases holding the titles, counted per release.
        let mut hits: HashMap<String, HashSet<usize>> = HashMap::new();
        for (index, track) in tracks.iter().enumerate().take(ALBUM_TITLE_SEARCHES) {
            let query = lucene(&[("recording", Some(track.title)), ("artist", Some(artist))]);
            let found: RecordingSearch = self.get("recording", &[("query", &query), ("limit", "25")]).await?;
            for recording in found.recordings {
                for release in recording.releases {
                    hits.entry(release.id).or_default().insert(index);
                }
            }
        }
        if let Some(album) = album.filter(|a| !a.trim().is_empty()) {
            let query = lucene(&[("release", Some(album)), ("artist", Some(artist))]);
            let found: ReleaseSearch = self.get("release", &[("query", &query), ("limit", "25")]).await?;
            for release in found.releases {
                hits.entry(release.id).or_default();
            }
        }
        let mut ranked: Vec<(String, usize)> = hits.into_iter().map(|(id, set)| (id, set.len())).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(ALBUM_LOOKUPS);

        let mut scored = Vec::new();
        for (id, _) in ranked {
            let release: Release = self.get(&format!("release/{id}"), &[("inc", "recordings+artist-credits+release-groups")]).await?;
            let detail = ReleaseDetail::from(&release);
            let matches = match_tracks(&tracks, &detail.tracks);
            if !matches.is_empty() {
                scored.push((detail, matches));
            }
        }
        Ok(rank_albums(scored, tracks.len() as i32))
    }

    /// One recording by MBID, with the album, track number and album artist
    /// taken from `mb_release_id` when given, else from its most album-like
    /// official release.
    pub async fn recording(&self, mb_recording_id: &str, mb_release_id: Option<&str>) -> anyhow::Result<RecordingDetail> {
        let recording: Recording = self
            .get(&format!("recording/{mb_recording_id}"), &[("inc", "artist-credits+releases+release-groups+genres+tags")])
            .await?;
        let release_id = match mb_release_id {
            Some(id) => Some(id.to_string()),
            None => preferred_release(&recording.releases, None).map(|r| r.id.clone()),
        };
        let release: Option<Release> = match &release_id {
            Some(id) => Some(self.get(&format!("release/{id}"), &[("inc", "recordings+artist-credits+release-groups")]).await?),
            None => None,
        };
        let track_number = release.as_ref().and_then(|r| {
            r.media
                .iter()
                .flat_map(|m| &m.tracks)
                .find(|t| t.recording.as_ref().is_some_and(|rec| rec.id == recording.id))
                .and_then(|t| t.number.parse().ok().or(Some(t.position)))
        });
        Ok(RecordingDetail {
            mb_recording_id: recording.id.clone(),
            title: recording.title.clone(),
            artist: credit_name(&recording.artist_credit),
            mb_artist_id: recording.artist_credit.first().map(|c| c.artist.id.clone()),
            album: release.as_ref().map(|r| r.title.clone()),
            mb_release_id: release.as_ref().map(|r| r.id.clone()),
            genre: top_genre(&recording.genres).or_else(|| top_genre(&recording.tags)),
            track_number,
            album_artist: release.as_ref().and_then(|r| credit_name(&r.artist_credit)),
            mb_release_group_id: release.as_ref().and_then(|r| r.release_group.as_ref().map(|g| g.id.clone())),
        })
    }
}

/// A Lucene query of quoted phrases, empty fields left out.
fn lucene(fields: &[(&str, Option<&str>)]) -> String {
    fields
        .iter()
        .filter_map(|(field, value)| {
            let value = (*value)?.trim();
            (!value.is_empty()).then(|| format!("{field}:\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn credit_name(credit: &[Credit]) -> Option<String> {
    let name: String = credit.iter().map(|c| format!("{}{}", c.name, c.joinphrase)).collect();
    (!name.trim().is_empty()).then(|| name.trim().to_string())
}

fn top_genre(genres: &[Genre]) -> Option<String> {
    genres.iter().max_by_key(|g| g.count).map(|g| g.name.clone())
}

fn year(date: Option<&str>) -> Option<i32> {
    date.and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
}

/// `ReleaseDetail::kind_penalty` for a release as a search lists it.
fn release_penalty(r: &ReleaseRef) -> u8 {
    let group = r.release_group.as_ref();
    ReleaseDetail {
        id: String::new(),
        group_id: String::new(),
        title: String::new(),
        artist: None,
        status: r.status.clone(),
        primary_type: group.and_then(|g| g.primary_type.clone()),
        secondary_types: group.map(|g| g.secondary_types.clone()).unwrap_or_default(),
        year: None,
        track_count: 0,
        has_front: false,
        tracks: Vec::new(),
    }
    .kind_penalty()
}

fn front_cover_url(release_id: &str) -> String {
    format!("https://coverartarchive.org/release/{release_id}/front-500")
}

/// The release a recording is best known by: the one whose title the caller
/// gave, else official before not, an album before anything else, earliest.
fn preferred_release<'a>(releases: &'a [ReleaseRef], album: Option<&str>) -> Option<&'a ReleaseRef> {
    let wanted = album.map(normalize);
    releases.iter().min_by_key(|r| {
        let named = wanted.as_ref().is_some_and(|w| normalize(&r.title) == *w);
        let group = r.release_group.as_ref();
        let album = group.and_then(|g| g.primary_type.as_deref()) == Some("Album");
        let plain = group.is_none_or(|g| g.secondary_types.is_empty());
        (
            !named,
            r.status.as_deref() != Some("Official"),
            !album,
            !plain,
            year(r.date.as_deref()).unwrap_or(i32::MAX),
        )
    })
}

// ── MusicBrainz JSON ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RecordingSearch {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Deserialize)]
struct ReleaseSearch {
    #[serde(default)]
    releases: Vec<ReleaseRef>,
}

#[derive(Deserialize)]
struct Recording {
    id: String,
    title: String,
    #[serde(default)]
    score: Option<u8>,
    #[serde(default)]
    length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<Credit>,
    #[serde(default)]
    releases: Vec<ReleaseRef>,
    #[serde(default)]
    genres: Vec<Genre>,
    #[serde(default)]
    tags: Vec<Genre>,
}

impl Recording {
    fn into_candidate(self, album: Option<&str>, duration_ms: Option<u64>) -> MetadataCandidate {
        let release = preferred_release(&self.releases, album);
        // The search's own score knows nothing about the file's length; a
        // candidate that is minutes off is another version of the song.
        let mut score = self.score.unwrap_or(0);
        if let (Some(want), Some(have)) = (duration_ms, self.length) {
            if (want as i64 - have as i64).abs() > DURATION_TOLERANCE_MS {
                score = score.saturating_sub(30);
            }
        }
        let track_number = release.and_then(|r| r.media.first()).and_then(|m| m.track.first()).and_then(|t| t.number.parse().ok());
        MetadataCandidate {
            mb_recording_id: self.id,
            title: self.title,
            artist: credit_name(&self.artist_credit),
            mb_artist_id: self.artist_credit.first().map(|c| c.artist.id.clone()),
            album: release.map(|r| r.title.clone()),
            mb_release_id: release.map(|r| r.id.clone()),
            duration_ms: self.length,
            genre: top_genre(&self.genres).or_else(|| top_genre(&self.tags)),
            track_number,
            year: release.and_then(|r| year(r.date.as_deref())),
            cover_art_url: release.map(|r| front_cover_url(&r.id)),
            score,
        }
    }
}

#[derive(Deserialize)]
struct Credit {
    name: String,
    #[serde(default)]
    joinphrase: String,
    artist: CreditArtist,
}

#[derive(Deserialize)]
struct CreditArtist {
    id: String,
}

#[derive(Deserialize)]
struct Genre {
    name: String,
    #[serde(default)]
    count: i64,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    id: String,
    #[serde(rename = "primary-type", default)]
    primary_type: Option<String>,
    #[serde(rename = "secondary-types", default)]
    secondary_types: Vec<String>,
}

/// A release as search results and recordings list it.
#[derive(Deserialize)]
struct ReleaseRef {
    id: String,
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "release-group", default)]
    release_group: Option<ReleaseGroup>,
    #[serde(default)]
    media: Vec<SearchMedium>,
}

#[derive(Deserialize)]
struct SearchMedium {
    #[serde(default)]
    track: Vec<SearchTrack>,
}

#[derive(Deserialize)]
struct SearchTrack {
    #[serde(default)]
    number: String,
}

/// A release looked up by id with its tracklist.
#[derive(Deserialize)]
struct Release {
    id: String,
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<Credit>,
    #[serde(rename = "release-group", default)]
    release_group: Option<ReleaseGroup>,
    #[serde(rename = "cover-art-archive", default)]
    cover_art_archive: Option<CoverArtArchive>,
    #[serde(default)]
    media: Vec<Medium>,
}

#[derive(Deserialize)]
struct CoverArtArchive {
    #[serde(default)]
    front: bool,
}

#[derive(Deserialize)]
struct Medium {
    #[serde(default)]
    position: i32,
    #[serde(default)]
    tracks: Vec<Track>,
}

#[derive(Deserialize)]
struct Track {
    #[serde(default)]
    number: String,
    #[serde(default)]
    position: i32,
    title: String,
    #[serde(default)]
    length: Option<i64>,
    #[serde(default)]
    recording: Option<TrackRecording>,
}

#[derive(Deserialize)]
struct TrackRecording {
    id: String,
}

// ── Album matching, as the metadata service does it ──────────────────────

struct ReleaseDetail {
    id: String,
    group_id: String,
    title: String,
    artist: Option<String>,
    status: Option<String>,
    primary_type: Option<String>,
    secondary_types: Vec<String>,
    year: Option<i32>,
    track_count: i32,
    has_front: bool,
    tracks: Vec<ListedTrack>,
}

impl From<&Release> for ReleaseDetail {
    fn from(r: &Release) -> Self {
        let tracks: Vec<ListedTrack> = r
            .media
            .iter()
            .flat_map(|m| {
                m.tracks.iter().filter_map(move |t| {
                    Some(ListedTrack {
                        recording_id: t.recording.as_ref()?.id.clone(),
                        title: t.title.clone(),
                        disc: m.position,
                        position: t.position,
                        length: t.length,
                    })
                })
            })
            .collect();
        let group = r.release_group.as_ref();
        Self {
            id: r.id.clone(),
            group_id: group.map(|g| g.id.clone()).unwrap_or_else(|| r.id.clone()),
            title: r.title.clone(),
            artist: credit_name(&r.artist_credit),
            status: r.status.clone(),
            primary_type: group.and_then(|g| g.primary_type.clone()),
            secondary_types: group.map(|g| g.secondary_types.clone()).unwrap_or_default(),
            year: year(r.date.as_deref()),
            track_count: tracks.len() as i32,
            has_front: r.cover_art_archive.as_ref().is_some_and(|c| c.front),
            tracks,
        }
    }
}

impl ReleaseDetail {
    fn official(&self) -> bool {
        self.status.as_deref() == Some("Official")
    }

    /// How far this is from "the album": a studio album scores 0.
    fn kind_penalty(&self) -> u8 {
        let mut p = 0;
        if !self.official() {
            p += 4;
        }
        if self.secondary_types.iter().any(|t| t == "Live") {
            p += 3;
        }
        if self
            .secondary_types
            .iter()
            .any(|t| matches!(t.as_str(), "Compilation" | "Soundtrack" | "Remix" | "DJ-mix" | "Mixtape/Street"))
        {
            p += 2;
        }
        if !matches!(self.primary_type.as_deref(), Some("Album") | Some("EP")) {
            p += 1;
        }
        p
    }
}

struct ListedTrack {
    recording_id: String,
    title: String,
    disc: i32,
    position: i32,
    length: Option<i64>,
}

/// One candidate per album (release group): the edition that fits best
/// stands for the rest, which are counted in `editions`.
fn rank_albums(mut scored: Vec<(ReleaseDetail, Vec<AlbumTrackMatch>)>, n: i32) -> Vec<AlbumCandidate> {
    scored.sort_by(|(a, am), (b, bm)| {
        bm.len()
            .cmp(&am.len())
            .then(b.official().cmp(&a.official()))
            .then((a.track_count - n).abs().cmp(&(b.track_count - n).abs()))
            .then(a.year.unwrap_or(i32::MAX).cmp(&b.year.unwrap_or(i32::MAX)))
            .then(a.id.cmp(&b.id))
    });
    let mut groups: Vec<(ReleaseDetail, Vec<AlbumTrackMatch>, u32, Option<String>)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (detail, matches) in scored {
        match seen.get(&detail.group_id) {
            Some(&i) => {
                groups[i].2 += 1;
                if groups[i].3.is_none() && detail.has_front {
                    groups[i].3 = Some(front_cover_url(&detail.id));
                }
            }
            None => {
                seen.insert(detail.group_id.clone(), groups.len());
                let cover = detail.has_front.then(|| front_cover_url(&detail.id));
                groups.push((detail, matches, 1, cover));
            }
        }
    }
    groups.sort_by(|(a, am, _, _), (b, bm, _, _)| {
        bm.len()
            .cmp(&am.len())
            .then(a.kind_penalty().cmp(&b.kind_penalty()))
            .then(a.year.unwrap_or(i32::MAX).cmp(&b.year.unwrap_or(i32::MAX)))
            .then(a.id.cmp(&b.id))
    });
    groups
        .into_iter()
        .take(MAX_ALBUM_CANDIDATES)
        .map(|(d, matches, editions, cover)| AlbumCandidate {
            cover_art_url: cover,
            mb_release_id: d.id,
            mb_release_group_id: d.group_id,
            title: d.title,
            artist: d.artist,
            year: d.year,
            primary_type: d.primary_type,
            secondary_types: d.secondary_types,
            status: d.status,
            track_count: d.track_count,
            matched: matches.len() as u32,
            editions,
            tracks: matches,
        })
        .collect()
}

const TYPOGRAPHIC: &[(char, char)] = &[
    ('\u{2019}', '\''),
    ('\u{2018}', '\''),
    ('\u{02BC}', '\''),
    ('\u{2032}', '\''),
    ('\u{0060}', '\''),
    ('\u{00B4}', '\''),
    ('\u{201C}', '"'),
    ('\u{201D}', '"'),
];

/// Titles compared the way people tag them: typographic punctuation folded,
/// case and punctuation ignored.
fn normalize(title: &str) -> String {
    title
        .chars()
        .map(|c| TYPOGRAPHIC.iter().find(|(from, _)| *from == c).map_or(c, |(_, to)| *to))
        .collect::<String>()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The title without a trailing "(Remastered 2014)" or "[Live]".
fn core(title: &str) -> String {
    let cut = title.find(['(', '[']).map_or(title, |i| &title[..i]);
    normalize(cut)
}

/// 2 for the same title, 1 for the same song under a variant of it, 0 for
/// anything else. A title that only starts like the other counts only when
/// the lengths agree ("Michael" is not "Michael Jackson Medley").
fn title_score(requested: &str, listed: &str, gap_ms: Option<i64>) -> u8 {
    let (r, l) = (normalize(requested), normalize(listed));
    if r == l {
        return 2;
    }
    if !core(requested).is_empty() && core(requested) == core(listed) {
        return 1;
    }
    let prefix = r.len().min(l.len()) >= 4 && (r.starts_with(&format!("{l} ")) || l.starts_with(&format!("{r} ")));
    if prefix && gap_ms.is_some_and(|g| g <= PREFIX_DURATION_MS) {
        return 1;
    }
    0
}

/// Each requested track to at most one listed track and back, best pairs
/// first, so a title listed twice goes to the file whose length fits.
fn match_tracks(requested: &[AlbumTrackQuery], listed: &[ListedTrack]) -> Vec<AlbumTrackMatch> {
    let mut pairs: Vec<(u8, i64, usize, usize)> = Vec::new();
    for (ri, r) in requested.iter().enumerate() {
        for (li, l) in listed.iter().enumerate() {
            let known_gap = match (r.duration_ms, l.length) {
                (Some(a), Some(b)) => Some((a as i64 - b).abs()),
                _ => None,
            };
            let score = title_score(r.title, &l.title, known_gap);
            if score > 0 {
                pairs.push((score, known_gap.unwrap_or(DURATION_TOLERANCE_MS), ri, li));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let (mut used_r, mut used_l) = (HashSet::new(), HashSet::new());
    let mut out: BTreeMap<usize, AlbumTrackMatch> = BTreeMap::new();
    for (_, _, ri, li) in pairs {
        if used_r.contains(&ri) || used_l.contains(&li) {
            continue;
        }
        used_r.insert(ri);
        used_l.insert(li);
        let l = &listed[li];
        out.insert(
            ri,
            AlbumTrackMatch {
                index: ri as u32,
                mb_recording_id: l.recording_id.clone(),
                title: l.title.clone(),
                disc: l.disc,
                position: l.position,
                duration_ms: l.length.map(|v| v as u64),
            },
        );
    }
    out.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(title: &str, ms: Option<u64>) -> AlbumTrackQuery<'_> {
        AlbumTrackQuery { title, duration_ms: ms }
    }

    fn listed(title: &str, position: i32, length: Option<i64>) -> ListedTrack {
        ListedTrack { recording_id: format!("rec-{position}"), title: title.into(), disc: 1, position, length }
    }

    #[test]
    fn queries_quote_and_escape() {
        assert_eq!(lucene(&[("recording", Some("Bohemian Rhapsody")), ("artist", Some("Queen")), ("release", None)]),
            r#"recording:"Bohemian Rhapsody" AND artist:"Queen""#);
        assert_eq!(lucene(&[("recording", Some(r#"Say "Hi" \o/"#))]), r#"recording:"Say \"Hi\" \\o/""#);
        assert_eq!(lucene(&[("recording", Some("  ")), ("artist", None)]), "");
    }

    #[test]
    fn titles_match_across_typography_case_and_suffixes() {
        assert_eq!(title_score("the prophet's song", "The Prophet\u{2019}s Song", None), 2);
        assert_eq!(title_score("Take Me Out (Remastered 2014)", "Take Me Out", None), 1);
        assert_eq!(title_score("Michael", "Jacqueline", None), 0);
        assert_eq!(title_score("Michael Jackson Medley", "Michael", Some(200_000)), 0);
    }

    #[test]
    fn a_repeated_title_goes_to_the_track_whose_length_fits() {
        let requested = [req("Love of My Life", Some(219_000))];
        let list = [listed("Love of My Life", 9, Some(219_500)), listed("Love of My Life", 14, Some(300_000))];
        let m = match_tracks(&requested, &list);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].position, 9);
    }

    #[test]
    fn parses_a_recording_search() {
        let json = r#"{"recordings":[{"id":"r1","score":100,"title":"Bohemian Rhapsody","length":354000,
          "artist-credit":[{"name":"Queen","joinphrase":"","artist":{"id":"a1","name":"Queen"}}],
          "releases":[
            {"id":"live","title":"Live Killers","status":"Official","date":"1979-06-22",
             "release-group":{"id":"g2","primary-type":"Album","secondary-types":["Live"]},
             "media":[{"position":1,"track":[{"id":"t","number":"5","title":"Bohemian Rhapsody"}]}]},
            {"id":"opera","title":"A Night at the Opera","status":"Official","date":"1975-11-21",
             "release-group":{"id":"g1","primary-type":"Album","secondary-types":[]},
             "media":[{"position":1,"track":[{"id":"t","number":"11","title":"Bohemian Rhapsody"}]}]}],
          "tags":[{"count":3,"name":"rock"}]}]}"#;
        let found: RecordingSearch = serde_json::from_str(json).unwrap();
        let c = found.recordings.into_iter().next().unwrap().into_candidate(None, Some(355_000));
        assert_eq!(c.album.as_deref(), Some("A Night at the Opera"));
        assert_eq!(c.track_number, Some(11));
        assert_eq!(c.year, Some(1975));
        assert_eq!(c.genre.as_deref(), Some("rock"));
        assert_eq!(c.score, 100);
    }

    /// Against musicbrainz.org itself: `cargo test live_musicbrainz -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_musicbrainz() {
        let mb = MusicBrainz::new(&MusicBrainzConfig::default()).unwrap();
        let found = mb.identify(Some("Bohemian Rhapsody"), Some("Queen"), None, Some(355_000), 5).await.unwrap();
        let top = &found[0];
        println!("identify: {} / {:?} / {:?} / {:?} / track {:?} / score {}", top.title, top.artist, top.album, top.year, top.track_number, top.score);
        assert!(top.artist.as_deref().unwrap_or("").contains("Queen"));

        let detail = mb.recording(&top.mb_recording_id, top.mb_release_id.as_deref()).await.unwrap();
        println!("recording: {:?} / {:?} / track {:?} / genre {:?} / group {:?}", detail.album, detail.album_artist, detail.track_number, detail.genre, detail.mb_release_group_id);
        assert!(detail.mb_release_group_id.is_some());

        let tracks = vec![
            AlbumTrackQuery { title: "Death on Two Legs (Dedicated to...)", duration_ms: Some(223_000) },
            AlbumTrackQuery { title: "Lazing on a Sunday Afternoon", duration_ms: Some(67_000) },
            AlbumTrackQuery { title: "I'm in Love With My Car", duration_ms: Some(185_000) },
            AlbumTrackQuery { title: "You're My Best Friend", duration_ms: Some(172_000) },
        ];
        let albums = mb.identify_album("Queen", Some("A Night at the Opera"), tracks).await.unwrap();
        for a in albums.iter().take(3) {
            println!("album: {} ({:?}) {:?}/{:?} matched {} of {} editions {}", a.title, a.year, a.primary_type, a.secondary_types, a.matched, a.track_count, a.editions);
        }
        assert_eq!(albums[0].title, "A Night at the Opera");
        assert_eq!(albums[0].matched, 4);
    }
}
