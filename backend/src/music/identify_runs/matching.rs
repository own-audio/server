// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a run proposes for a group of tracks, from what the source offered.
//! No I/O: the job feeds it candidates and writes what it returns.
//!
//! "Sure" is strict on purpose. Accepting every sure proposal is one click, and
//! the mistakes found tagging a real library by hand all had a recognisable
//! shape — a single-disc rip matched to a two-disc release, two files on one
//! track, a compilation song moved to its original album — so each of those is
//! a reason to look first, not a guess to make.

use crate::metadata::public::normalize;
use crate::metadata::{AlbumCandidate, MetadataCandidate, RecordingDetail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A track as the run sees it.
#[derive(Debug, Clone)]
pub struct FileTrack {
    pub id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Sure,
    Review,
    None,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Sure => "sure",
            Class::Review => "review",
            Class::None => "none",
        }
    }
}

/// Everything an accept writes, so accepting needs no lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProposedTrack {
    pub mb_recording_id: String,
    pub title: String,
    /// The recording's artist when the source said; else the file's stays.
    pub artist: Option<String>,
    pub mb_artist_id: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub year: Option<i32>,
    pub length_ms: Option<u64>,
    /// 0–100, the source's confidence for a single track; 100 for an album match.
    pub score: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub track_id: Uuid,
    pub class: Class,
    pub reasons: Vec<&'static str>,
    pub proposal: Option<ProposedTrack>,
    /// Set only the recording and keep the file's album (see `album_differs`).
    pub keep_album: bool,
}

/// The candidate a group should start on: most tracks matched, then a release
/// with exactly as many tracks as the group, then one whose discs and track
/// numbers are the files' own, then the source's order.
pub fn choose(files: &[FileTrack], candidates: &[AlbumCandidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .max_by_key(|(i, c)| {
            (
                c.matched,
                c.track_count as usize == files.len(),
                layout_mismatches(files, c) == 0,
                std::cmp::Reverse(*i),
            )
        })
        .map(|(i, _)| i)
}

/// Files whose disc and track number differ from where the release puts them.
/// A file with no track number has nothing to contradict.
fn layout_mismatches(files: &[FileTrack], c: &AlbumCandidate) -> usize {
    c.tracks
        .iter()
        .filter(|m| {
            files.get(m.index as usize).is_some_and(|f| {
                f.track_number.is_some_and(|n| (f.disc_number.unwrap_or(1), n) != (m.disc, m.position))
            })
        })
        .count()
}

/// Proposals for an album group on candidate `chosen`, plus the indexes of files
/// the release does not hold, for single-track lookups.
pub fn album_proposals(
    files: &[FileTrack],
    candidates: &[AlbumCandidate],
    chosen: usize,
) -> (Vec<Proposal>, Vec<usize>) {
    let Some(c) = candidates.get(chosen) else {
        return (Vec::new(), (0..files.len()).collect());
    };
    let n = files.len();

    let mut group_reasons: Vec<&'static str> = Vec::new();
    if c.track_count as usize != n {
        // The files are part of the release, or more than it.
        group_reasons.push("incomplete");
    }
    if (c.matched as usize) < n {
        group_reasons.push("partial");
    }
    // Another release group is a rival only when it is as good on both counts:
    // the box sets and collections that hold every song of an album (a 43-track
    // vinyl box, a 163-track collection) are not, when the album has exactly as
    // many tracks as the files.
    let exact = |x: &AlbumCandidate| x.track_count as usize == n;
    let rival = candidates.iter().enumerate().any(|(i, o)| {
        i != chosen
            && o.mb_release_group_id != c.mb_release_group_id
            && o.matched >= c.matched
            && (exact(o) || !exact(c))
    });
    if rival {
        group_reasons.push("ambiguous");
    }
    let tagged_album = files.iter().find_map(|f| f.album.as_deref());
    let album_differs = tagged_album.is_some_and(|a| !same_title(a, &c.title));
    if album_differs {
        group_reasons.push("album_differs");
    }

    let mut taken: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    for m in &c.tracks {
        *taken.entry((m.disc, m.position)).or_default() += 1;
    }

    let mut matched = vec![false; n];
    let mut out = Vec::new();
    for m in &c.tracks {
        let Some(f) = files.get(m.index as usize) else { continue };
        matched[m.index as usize] = true;
        let mut reasons = group_reasons.clone();
        if f.track_number.is_some_and(|t| (f.disc_number.unwrap_or(1), t) != (m.disc, m.position)) {
            reasons.push("layout");
        }
        if taken.get(&(m.disc, m.position)).copied().unwrap_or(0) > 1 {
            reasons.push("duplicate");
        }
        out.push(Proposal {
            track_id: f.id,
            class: if reasons.is_empty() { Class::Sure } else { Class::Review },
            reasons,
            proposal: Some(ProposedTrack {
                mb_recording_id: m.mb_recording_id.clone(),
                title: m.title.clone(),
                artist: None,
                mb_artist_id: None,
                album: Some(c.title.clone()),
                album_artist: c.artist.clone(),
                mb_release_id: Some(c.mb_release_id.clone()),
                mb_release_group_id: Some(c.mb_release_group_id.clone()),
                track_number: Some(m.position),
                disc_number: Some(m.disc),
                year: c.year,
                length_ms: m.duration_ms,
                score: 100,
            }),
            keep_album: album_differs,
        });
    }
    let leftovers = (0..n).filter(|i| !matched[*i]).collect();
    (out, leftovers)
}

/// The proposal for one track from a recording search, best candidate first.
pub fn track_proposal(file: &FileTrack, found: &[MetadataCandidate]) -> Proposal {
    let none = || Proposal {
        track_id: file.id,
        class: Class::None,
        reasons: vec!["not_found"],
        proposal: None,
        keep_album: false,
    };
    let Some(top) = found.first() else { return none() };
    if top.score < 50 {
        return none();
    }
    let mut reasons: Vec<&'static str> = Vec::new();
    if top.score < 90 {
        reasons.push("weak");
    }
    let gap = match (file.duration_ms, top.duration_ms) {
        (Some(a), Some(b)) => Some(a.abs_diff(b)),
        _ => None,
    };
    if gap.is_some_and(|g| g > 5_000) {
        reasons.push("weak");
        reasons.dedup();
    }
    // The source's scores are relative — its best hit is 100 however poor — so
    // the artist is checked here: a search for "X 4" by Metallica (a bad tag
    // for "2 X 4") came back as another band's "X 4" at 100.
    if let (Some(a), Some(b)) = (file.artist.as_deref(), top.artist.as_deref()) {
        if !same_artist(a, b) {
            reasons.push("artist_differs");
        }
    }
    // A song from a compilation matched to its original album: keep the file's
    // album by default (decided 2026-10-11) and let the owner switch.
    let album_differs = match (file.album.as_deref(), top.album.as_deref()) {
        (Some(a), Some(b)) => !same_title(a, b),
        _ => false,
    };
    if album_differs {
        reasons.push("album_differs");
    }
    // A song is never sure from a search alone when a near-equal rival exists:
    // the same recording title by the same artist on another release.
    if found.get(1).is_some_and(|second| top.score.saturating_sub(second.score) < 5
        && second.mb_recording_id != top.mb_recording_id)
    {
        reasons.push("ambiguous");
    }
    Proposal {
        track_id: file.id,
        class: if reasons.is_empty() { Class::Sure } else { Class::Review },
        reasons,
        proposal: Some(ProposedTrack {
            mb_recording_id: top.mb_recording_id.clone(),
            title: top.title.clone(),
            artist: top.artist.clone(),
            mb_artist_id: top.mb_artist_id.clone(),
            album: top.album.clone(),
            album_artist: None,
            mb_release_id: top.mb_release_id.clone(),
            mb_release_group_id: None,
            track_number: top.track_number,
            disc_number: None,
            year: top.year,
            length_ms: top.duration_ms,
            score: top.score,
        }),
        keep_album: album_differs,
    }
}

/// The proposal for a recording the owner picked: what MusicBrainz says of it
/// by id, with the year and length a search found for it where it did.
pub fn chosen(recording: RecordingDetail, found: Option<&MetadataCandidate>) -> ProposedTrack {
    let found = found.filter(|c| c.mb_recording_id == recording.mb_recording_id);
    ProposedTrack {
        mb_recording_id: recording.mb_recording_id,
        title: recording.title,
        artist: recording.artist,
        mb_artist_id: recording.mb_artist_id,
        album: recording.album,
        album_artist: recording.album_artist,
        mb_release_id: recording.mb_release_id,
        mb_release_group_id: recording.mb_release_group_id,
        track_number: recording.track_number,
        disc_number: None,
        year: found.and_then(|c| c.year),
        length_ms: found.and_then(|c| c.duration_ms),
        score: 100,
    }
}

/// A MusicBrainz id of `kind` ("recording", "release") from what someone
/// pasted: the bare id, or a musicbrainz.org link to it.
pub fn mbid(input: &str, kind: &str) -> Option<String> {
    let s = input.trim();
    let id = match s.find(&format!("/{kind}/")) {
        Some(at) => s[at + kind.len() + 2..].split(['/', '?', '#']).next()?,
        None if !s.contains('/') => s,
        None => return None,
    };
    Uuid::parse_str(id).ok().map(|u| u.to_string())
}

/// The same artist, or one credit inside the other ("Wings" in "Paul McCartney
/// & Wings").
fn same_artist(a: &str, b: &str) -> bool {
    let (x, y) = (normalize(a), normalize(b));
    !x.is_empty() && !y.is_empty() && (x == y || format!(" {y} ").contains(&format!(" {x} ")) || format!(" {x} ").contains(&format!(" {y} ")))
}

/// Two album titles the same once case, punctuation and a trailing
/// "(Remastered)" or "[Deluxe]" are set aside.
fn same_title(a: &str, b: &str) -> bool {
    let cut = |t: &str| normalize(t.find(['(', '[']).map_or(t, |i| &t[..i]));
    let (x, y) = (cut(a), cut(b));
    !x.is_empty() && x == y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::AlbumTrackMatch;

    fn file(n: i32, title: &str) -> FileTrack {
        FileTrack {
            id: Uuid::from_u128(n as u128),
            title: title.into(),
            artist: Some("Bob Dylan".into()),
            album: Some("Blonde on Blonde".into()),
            track_number: Some(n),
            disc_number: None,
            duration_ms: None,
        }
    }

    fn release(group: &str, discs: &[i32], matched: u32) -> AlbumCandidate {
        // `discs`: tracks per disc, matched in order to files 0..
        let mut tracks = Vec::new();
        let mut index = 0;
        for (d, count) in discs.iter().enumerate() {
            for p in 1..=*count {
                if index < matched {
                    tracks.push(AlbumTrackMatch {
                        index,
                        mb_recording_id: format!("rec-{index}"),
                        title: format!("Song {index}"),
                        disc: d as i32 + 1,
                        position: p,
                        duration_ms: None,
                    });
                }
                index += 1;
            }
        }
        AlbumCandidate {
            mb_release_id: format!("{group}-{}", discs.len()),
            mb_release_group_id: group.into(),
            title: "Blonde on Blonde".into(),
            artist: Some("Bob Dylan".into()),
            year: Some(1966),
            primary_type: Some("Album".into()),
            secondary_types: vec![],
            status: Some("Official".into()),
            track_count: discs.iter().sum(),
            matched,
            editions: 2,
            cover_art_url: None,
            tracks,
        }
    }

    fn files(n: i32) -> Vec<FileTrack> {
        (1..=n).map(|i| file(i, &format!("Song {i}"))).collect()
    }

    #[test]
    fn a_one_disc_rip_prefers_the_one_disc_edition() {
        // Blonde on Blonde: the 2-LP original first, the 1-CD edition second.
        let cands = [release("bob", &[8, 6], 14), release("bob", &[14], 14)];
        let fs = files(14);
        assert_eq!(choose(&fs, &cands), Some(1));
        let (props, left) = album_proposals(&fs, &cands, 1);
        assert!(left.is_empty());
        assert!(props.iter().all(|p| p.class == Class::Sure), "{props:?}");
    }

    #[test]
    fn the_wrong_layout_is_a_reason_to_look() {
        let cands = [release("bob", &[8, 6], 14)];
        let (props, _) = album_proposals(&files(14), &cands, 0);
        let layout = props.iter().filter(|p| p.reasons.contains(&"layout")).count();
        assert_eq!(layout, 6, "disc 2's six tracks are 9–14 on the files");
        assert!(props.iter().filter(|p| p.class == Class::Sure).count() == 8);
    }

    #[test]
    fn part_of_an_album_is_reviewed_and_the_rest_goes_track_by_track() {
        let cands = [release("bob", &[14], 4)];
        let (props, left) = album_proposals(&files(5), &cands, 0);
        assert_eq!(props.len(), 4);
        assert!(props.iter().all(|p| p.reasons.contains(&"incomplete") && p.reasons.contains(&"partial")));
        assert_eq!(left, vec![4]);
    }

    #[test]
    fn another_album_matching_as_well_makes_it_ambiguous() {
        let mut other = release("other", &[3], 3);
        other.title = "Greatest Hits".into();
        let cands = [release("bob", &[3], 3), other];
        let (props, _) = album_proposals(&files(3), &cands, 0);
        assert!(props.iter().all(|p| p.reasons.contains(&"ambiguous")));
    }

    #[test]
    fn a_box_set_holding_the_album_is_no_rival() {
        // What the hosted mirror answers for Master of Puppets: the album, then a
        // 43-track box and a 163-track collection that hold all eight songs too.
        let mut box_set = release("box", &[43], 8);
        box_set.title = "Limited Edition Vinyl Box Set".into();
        let mut collection = release("coll", &[163], 8);
        collection.title = "The Metallica Collection".into();
        let cands = [release("bob", &[8], 8), box_set, collection];
        let (props, _) = album_proposals(&files(8), &cands, 0);
        assert!(props.iter().all(|p| p.class == Class::Sure), "{:?}", props[0].reasons);
    }

    #[test]
    fn two_files_on_one_track_are_duplicates() {
        let mut c = release("bob", &[2], 2);
        c.tracks[1].position = 1; // both files matched to track 1
        let (props, _) = album_proposals(&files(2), &[c], 0);
        assert!(props.iter().all(|p| p.reasons.contains(&"duplicate")));
    }

    fn found(score: u8, album: &str, id: &str) -> MetadataCandidate {
        MetadataCandidate {
            mb_recording_id: id.into(),
            title: "Bluebird".into(),
            artist: Some("Bonnie Raitt".into()),
            mb_artist_id: None,
            album: Some(album.into()),
            mb_release_id: Some("rel".into()),
            duration_ms: Some(209_000),
            genre: None,
            track_number: Some(1),
            year: Some(1971),
            cover_art_url: None,
            score,
        }
    }

    fn bluebird() -> FileTrack {
        let mut f = file(1, "Bluebird");
        f.artist = Some("Bonnie Raitt".into());
        f
    }

    #[test]
    fn another_artist_is_never_sure() {
        let mut f = file(2, "X 4");
        f.artist = Some("Metallica".into());
        f.album = None;
        let mut other = found(100, "Industry", "a");
        other.artist = Some("Guy Boulanger".into());
        let p = track_proposal(&f, &[other]);
        assert_eq!(p.class, Class::Review);
        assert!(p.reasons.contains(&"artist_differs"));
        assert!(same_artist("Wings", "Paul McCartney & Wings"));
        assert!(!same_artist("Metallica", "Guy Boulanger"));
    }

    #[test]
    fn a_compilation_song_keeps_its_album_and_is_reviewed() {
        let mut f = bluebird();
        f.album = Some("'70s Soft Rock Essentials".into());
        f.duration_ms = Some(209_100);
        let p = track_proposal(&f, &[found(100, "Bonnie Raitt (Remastered)", "a")]);
        assert_eq!(p.class, Class::Review);
        assert!(p.keep_album && p.reasons == vec!["album_differs"]);
    }

    #[test]
    fn a_clean_single_is_sure_and_nothing_is_none() {
        let mut f = bluebird();
        f.album = Some("Bonnie Raitt".into());
        f.duration_ms = Some(209_100);
        let p = track_proposal(&f, &[found(100, "Bonnie Raitt (Remastered)", "a"), found(80, "Live", "b")]);
        assert_eq!((p.class, p.keep_album), (Class::Sure, false), "{:?}", p.reasons);
        assert_eq!(track_proposal(&f, &[]).class, Class::None);
        assert_eq!(track_proposal(&f, &[found(40, "x", "a")]).class, Class::None);
    }

    #[test]
    fn near_equal_rival_recordings_are_reviewed() {
        let mut f = bluebird();
        f.album = None;
        let p = track_proposal(&f, &[found(100, "A", "a"), found(98, "B", "b")]);
        assert!(p.reasons.contains(&"ambiguous"));
    }

    #[test]
    fn a_pasted_link_or_bare_id_gives_the_recording() {
        let id = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
        assert_eq!(mbid(id, "recording").as_deref(), Some(id));
        assert_eq!(mbid(&format!(" https://musicbrainz.org/recording/{id} "), "recording").as_deref(), Some(id));
        assert_eq!(mbid(&format!("https://musicbrainz.org/recording/{}/edit?x=1", id.to_uppercase()), "recording").as_deref(), Some(id));
        assert_eq!(mbid(&format!("https://musicbrainz.org/release/{id}"), "recording"), None);
        assert_eq!(mbid("track 02", "recording"), None);
    }
}
