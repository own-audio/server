// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identify runs: many tracks matched against MusicBrainz in the background,
//! the results kept as proposals for the owner to accept, reject or change.
//! Accepting writes the database through the same apply as a single identify,
//! so it is revertible the same way; no file is ever written.
//!
//! A run is worked off by the `music_identify_batch` job a few groups at a
//! time. Each step queues the next behind every other job already waiting, so
//! households take turns instead of one big library holding the source for
//! hours, and a refusal from the source (musicbrainz.org's "busy") pauses the
//! run with a growing delay instead of failing it.

pub mod matching;

use crate::app::AppConfig;
use crate::db;
use crate::hooks::Hooks;
use crate::metadata::{Identify, mirror::AlbumTrackQuery};
use anyhow::Context;
use matching::{Class, FileTrack, Proposal};
use serde_json::json;
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

pub const JOB_TYPE: &str = "music_identify_batch";

/// Groups looked up per job step: small enough that other households get a
/// turn soon, large enough that a fast private mirror is not slowed by queueing.
const STEP: i64 = 5;
/// Backoff levels before a run gives up: 5 s doubling to the 5-minute cap is
/// about half an hour of an unreachable source.
const MAX_BACKOFF_LEVEL: i16 = 10;
/// Candidates per single-track search.
const TRACK_CANDIDATES: usize = 5;

/// Queue the run's first step.
pub async fn start(pool: &PgPool, run_id: Uuid) -> anyhow::Result<()> {
    enqueue_step(pool, run_id, None).await
}

async fn enqueue_step(pool: &PgPool, run_id: Uuid, delay: Option<Duration>) -> anyhow::Result<()> {
    let at = delay.map(|d| chrono::Utc::now() + chrono::Duration::from_std(d).unwrap_or_default());
    db::jobs::enqueue(pool, JOB_TYPE, Some(&json!({ "run_id": run_id })), at).await?;
    Ok(())
}

/// One step of a run.
pub async fn execute(pool: &PgPool, config: &AppConfig, hooks: &dyn Hooks, job: &crate::jobs::models::Job) -> anyhow::Result<()> {
    let run_id: Uuid = job
        .payload
        .as_ref()
        .and_then(|p| p.get("run_id"))
        .and_then(|v| v.as_str())
        .context("missing run_id in payload")?
        .parse()?;
    let Some(run) = db::identify::find_run(pool, run_id).await? else { return Ok(()) };
    if matches!(run.status.as_str(), "ready" | "failed" | "cancelled") {
        return Ok(());
    }

    let Some(source) = Identify::from_config(config) else {
        hooks.identify_run_end(pool, run_id).await?;
        db::identify::set_status(pool, run_id, "failed", Some("no MusicBrainz source is configured")).await?;
        return Ok(());
    };
    let source = source?;

    if run.status == "queued" {
        plan_groups(pool, &run).await?;
    }
    db::identify::set_status(pool, run_id, "running", None).await?;

    for group in db::identify::pending_groups(pool, run_id, STEP).await? {
        if let Err(e) = look_up(pool, &source, &run, &group).await {
            // The source refused or is unreachable: pause, then try the same group again.
            let level = run.backoff_level + 1;
            if level > MAX_BACKOFF_LEVEL {
                hooks.identify_run_end(pool, run_id).await?;
                db::identify::set_status(pool, run_id, "failed", Some(&format!("{e:#}"))).await?;
                return Ok(());
            }
            tracing::warn!(%run_id, level, "identify source refused, waiting: {e:#}");
            db::identify::set_status(pool, run_id, "waiting", Some(&format!("{e:#}"))).await?;
            enqueue_step(pool, run_id, Some(backoff(level))).await?;
            return Ok(());
        }
    }

    if db::identify::pending_groups(pool, run_id, 1).await?.is_empty() {
        hooks.identify_run_end(pool, run_id).await?;
        db::identify::set_status(pool, run_id, "ready", None).await?;
    } else {
        enqueue_step(pool, run_id, None).await?;
    }
    Ok(())
}

fn backoff(level: i16) -> Duration {
    Duration::from_secs((5u64 << (level.clamp(1, 7) - 1) as u32).min(300))
}

/// Split the scope into groups and store them (see [`cluster`]).
async fn plan_groups(pool: &PgPool, run: &db::identify::Run) -> anyhow::Result<()> {
    let tracks = db::identify::scope_tracks(pool, run).await?;
    let groups = cluster(&tracks);
    let groups: Vec<(&str, String, Vec<Uuid>)> = groups.iter().map(|(k, l, ids)| (*k, l.clone(), ids.clone())).collect();
    db::identify::insert_groups(pool, run.id, &groups, tracks.len() as i32).await
}

/// An album's name as files spell it, made comparable: case, "Disc 2" and
/// "CD1", bracketed notes ("[FLAC]", "(Remastered)") and punctuation aside.
fn album_norm(album: &str) -> String {
    let lower = album.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut depth = 0usize;
    for c in lower.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(if c.is_alphanumeric() { c } else { ' ' }),
            _ => {}
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let mut kept = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let disc_word = matches!(w, "disc" | "disk" | "cd");
        const SPELT: [&str; 10] = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
        if disc_word && words.get(i + 1).is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()) || SPELT.contains(n)) {
            i += 2;
            continue;
        }
        // "cd1", "disc2" written as one word.
        if ["cd", "disc", "disk"].iter().any(|p| w.strip_prefix(p).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))) {
            i += 1;
            continue;
        }
        kept.push(w);
        i += 1;
    }
    kept.join(" ")
}

/// A song's directory, with a disc's own subdirectory ("CD1", "Disc 2") folded
/// into the album's: one album, one cluster.
fn folder_norm(folder: &str) -> String {
    let Some((parent, last)) = folder.rsplit_once('/') else { return folder.to_string() };
    let l = last.to_lowercase().replace([' ', '_', '-', '.'], "");
    let disc = ["cd", "disc", "disk"]
        .iter()
        .any(|p| l.strip_prefix(p).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())));
    if disc { parent.to_string() } else { folder.to_string() }
}

/// The scope's songs in groups, each looked up once, as MusicBrainz Picard
/// clusters before a lookup:
///
/// - songs of one folder (a disc's subfolder counted in its album's) whose
///   album names match once [`album_norm`]ed are one album, even when they
///   ended up on different album entities: a folder tagged partly with release
///   ids, an album spelt three ways;
/// - a song without an album tag joins its folder's album when that folder
///   holds exactly one;
/// - an uploaded song (no folder) is grouped by its album entity, as before,
///   so two copies of an album are never merged;
/// - a group of one is looked up as that song: an album search on one title
///   finds any record that holds it.
///
/// Albums first, by label; then the songs on their own.
fn cluster(tracks: &[db::identify::ScopeTrack]) -> Vec<(&'static str, String, Vec<Uuid>)> {
    let mut albums: BTreeMap<String, Vec<&db::identify::ScopeTrack>> = BTreeMap::new();
    let mut untagged: BTreeMap<String, Vec<&db::identify::ScopeTrack>> = BTreeMap::new();
    let mut singles: Vec<&db::identify::ScopeTrack> = Vec::new();
    for t in tracks {
        let album = t.album.as_deref().map(album_norm).filter(|a| !a.is_empty());
        match (&t.folder, album, t.album_id) {
            (Some(folder), Some(album), _) => albums.entry(format!("dir:{}\u{1f}{album}", folder_norm(folder))).or_default().push(t),
            (Some(folder), None, _) => untagged.entry(folder_norm(folder)).or_default().push(t),
            (None, Some(_), Some(id)) => albums.entry(format!("id:{id}")).or_default().push(t),
            _ => singles.push(t),
        }
    }
    for (folder, songs) in untagged {
        let prefix = format!("dir:{folder}\u{1f}");
        let mut in_folder = albums.keys().filter(|k| k.starts_with(&prefix));
        match (in_folder.next().cloned(), in_folder.next()) {
            (Some(only), None) => albums.get_mut(&only).expect("listed").extend(songs),
            _ => singles.extend(songs),
        }
    }

    let mut groups: Vec<(&'static str, String, Vec<Uuid>)> = Vec::new();
    for songs in albums.into_values() {
        if songs.len() < 2 {
            singles.extend(songs);
            continue;
        }
        let mut songs = songs;
        songs.sort_by(|a, b| (a.disc_number, a.track_number, &a.title).cmp(&(b.disc_number, b.track_number, &b.title)));
        let album = most_common(songs.iter().map(|t| t.album.as_deref())).unwrap_or_default();
        let by = most_common(songs.iter().map(|t| t.album_artist.as_deref().or(t.artist.as_deref())));
        let label = match by {
            Some(by) => format!("{album} — {by}"),
            None => album,
        };
        groups.push(("album", label, songs.iter().map(|t| t.id).collect()));
    }
    groups.sort_by_key(|g| g.1.to_lowercase());
    groups.extend(singles.into_iter().map(|t| ("track", t.title.clone(), vec![t.id])));
    groups
}

fn file_track(t: &db::identify::ScopeTrack) -> FileTrack {
    FileTrack {
        id: t.id,
        title: t.title.clone(),
        artist: t.artist.clone(),
        album: t.album.clone(),
        track_number: t.track_number,
        disc_number: t.disc_number,
        duration_ms: t.duration_secs.map(|s| s as u64 * 1000),
    }
}

/// Look one group up and store what it proposes.
async fn look_up(
    pool: &PgPool,
    source: &Identify<'_>,
    run: &db::identify::Run,
    group: &db::identify::Group,
) -> anyhow::Result<()> {
    let found = db::identify::tracks_by_id(pool, run.user_id, &group.track_ids).await?;
    // The group's own order; a track deleted since the run started is left out.
    let files: Vec<FileTrack> = group
        .track_ids
        .iter()
        .filter_map(|id| found.iter().find(|t| t.id == *id).map(file_track))
        .collect();

    let mut proposals: Vec<Proposal> = Vec::new();
    let mut candidates = Vec::new();
    let mut chosen = None;
    let mut singles: Vec<usize> = (0..files.len()).collect();

    let artist = most_common(found.iter().map(|t| t.album_artist.as_deref().or(t.artist.as_deref())));
    // A cluster can hold songs whose album is spelt differently, or not at all:
    // the name most of them carry is the one to search for.
    let album = most_common(files.iter().map(|f| f.album.as_deref()));
    if let (true, Some(artist), Some(album)) = (group.kind == "album", artist, album) {
        let queries: Vec<AlbumTrackQuery> =
            files.iter().map(|f| AlbumTrackQuery { title: &f.title, duration_ms: f.duration_ms }).collect();
        candidates = source.identify_album(&artist, Some(&album), queries).await?;
        // An album that holds under half the group is not what the group is —
        // an unofficial compilation's songs each come from another record — so
        // its songs are looked up one by one instead.
        let best = matching::choose(&files, &candidates).filter(|c| candidates[*c].matched as usize * 2 >= files.len());
        if let Some(c) = best {
            let (props, left) = matching::album_proposals(&files, &candidates, c);
            proposals = props;
            singles = left;
            chosen = Some(c as i16);
        }
    }
    // What each single lookup found, kept as the track's other recordings.
    let mut alternatives: std::collections::HashMap<Uuid, serde_json::Value> = std::collections::HashMap::new();
    for i in singles {
        let f = &files[i];
        let found = source
            .identify(Some(&f.title), f.artist.as_deref(), f.album.as_deref(), f.duration_ms, TRACK_CANDIDATES)
            .await?;
        proposals.push(matching::track_proposal(f, &found));
        alternatives.insert(f.id, serde_json::to_value(&found)?);
    }

    let stored: Vec<db::identify::NewProposal> = proposals
        .iter()
        .map(|p| db::identify::NewProposal {
            track_id: p.track_id,
            class: p.class.as_str(),
            reasons: p.reasons.clone(),
            proposal: p.proposal.as_ref().map(|x| serde_json::to_value(x).unwrap_or_default()),
            keep_album: p.keep_album,
            alternatives: alternatives.remove(&p.track_id).unwrap_or_else(|| serde_json::json!([])),
        })
        .collect();
    db::identify::save_group(pool, group, &serde_json::to_value(&candidates)?, chosen, &stored).await
}

fn most_common<'a>(values: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for v in values.flatten().filter(|v| !v.trim().is_empty()) {
        *counts.entry(v).or_default() += 1;
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v.to_string())
}

/// Re-map a looked-up album group onto another of its candidates: the owner
/// chose another edition. Decided proposals stay as they are, and so do the
/// owner's own picks.
pub async fn choose_edition(pool: &PgPool, run: &db::identify::Run, group_id: Uuid, index: usize) -> anyhow::Result<bool> {
    let Some(group) = db::identify::find_group(pool, group_id).await? else { return Ok(false) };
    if group.run_id != run.id || group.kind != "album" {
        return Ok(false);
    }
    let candidates: Vec<crate::metadata::AlbumCandidate> = serde_json::from_value(group.candidates.clone())?;
    if index >= candidates.len() {
        return Ok(false);
    }
    let files = group_files(pool, run, &group).await?;
    put_on_edition(pool, &group, &files, &candidates, index).await?;
    Ok(true)
}

/// Search an album group's editions again with the owner's words — the tags'
/// album or artist is what is wrong — and put the group on the best of them.
/// `None` for a group that is not an album of this run; an empty list when
/// nothing was found, and then nothing changes.
pub async fn search_group(
    pool: &PgPool,
    source: &Identify<'_>,
    run: &db::identify::Run,
    group_id: Uuid,
    artist: Option<&str>,
    album: Option<&str>,
) -> anyhow::Result<Option<usize>> {
    let Some(group) = db::identify::find_group(pool, group_id).await? else { return Ok(None) };
    if group.run_id != run.id || group.kind != "album" {
        return Ok(None);
    }
    let found = db::identify::tracks_by_id(pool, run.user_id, &group.track_ids).await?;
    let files = group_files(pool, run, &group).await?;
    let tagged = most_common(found.iter().map(|t| t.album_artist.as_deref().or(t.artist.as_deref())));
    let Some(artist) = artist.map(str::to_string).or(tagged) else { return Ok(Some(0)) };
    let album = album.map(str::to_string).or_else(|| files.first().and_then(|f| f.album.clone()));
    let queries: Vec<AlbumTrackQuery> =
        files.iter().map(|f| AlbumTrackQuery { title: &f.title, duration_ms: f.duration_ms }).collect();
    let candidates = source.identify_album(&artist, album.as_deref(), queries).await?;
    let Some(best) = matching::choose(&files, &candidates) else { return Ok(Some(0)) };
    let mut group = group;
    group.candidates = serde_json::to_value(&candidates)?;
    put_on_edition(pool, &group, &files, &candidates, best).await?;
    Ok(Some(candidates.len()))
}

async fn group_files(pool: &PgPool, run: &db::identify::Run, group: &db::identify::Group) -> anyhow::Result<Vec<FileTrack>> {
    let found = db::identify::tracks_by_id(pool, run.user_id, &group.track_ids).await?;
    Ok(group
        .track_ids
        .iter()
        .filter_map(|id| found.iter().find(|t| t.id == *id).map(file_track))
        .collect())
}

/// The group's pending proposals on candidate `index`, stored with the group's
/// candidates.
async fn put_on_edition(
    pool: &PgPool,
    group: &db::identify::Group,
    files: &[FileTrack],
    candidates: &[crate::metadata::AlbumCandidate],
    index: usize,
) -> anyhow::Result<()> {
    let (props, left) = matching::album_proposals(files, candidates, index);
    let mut stored: Vec<db::identify::NewProposal> = props
        .iter()
        .map(|p| db::identify::NewProposal {
            track_id: p.track_id,
            class: p.class.as_str(),
            reasons: p.reasons.clone(),
            proposal: p.proposal.as_ref().map(|x| serde_json::to_value(x).unwrap_or_default()),
            keep_album: p.keep_album,
            alternatives: serde_json::json!([]),
        })
        .collect();
    // Files this edition does not hold: nothing proposed on it.
    for i in left {
        stored.push(db::identify::NewProposal {
            track_id: files[i].id,
            class: Class::Review.as_str(),
            reasons: vec!["not_on_release"],
            proposal: None,
            keep_album: false,
            alternatives: serde_json::json!([]),
        });
    }
    db::identify::save_group(pool, group, &group.candidates, Some(index as i16), &stored).await
}

#[derive(Debug, PartialEq, Eq)]
pub enum AcceptOutcome {
    Accepted,
    /// Nothing to apply (a "none" proposal, or one without a match).
    NothingProposed,
    AlreadyDecided,
}

/// Apply one proposal: the same database write a single identify does, carrying
/// the run so the run can be reverted as one. The file's genre and, for
/// `keep_album`, its album, album artist and track number stay.
pub async fn accept(pool: &PgPool, user_id: Uuid, proposal_id: Uuid) -> anyhow::Result<AcceptOutcome> {
    let Some(p) = db::identify::find_proposal(pool, proposal_id).await? else {
        return Ok(AcceptOutcome::NothingProposed);
    };
    if p.decision != "pending" {
        return Ok(AcceptOutcome::AlreadyDecided);
    }
    let Some(proposed) = p
        .proposal
        .as_ref()
        .and_then(|v| serde_json::from_value::<matching::ProposedTrack>(v.clone()).ok())
    else {
        return Ok(AcceptOutcome::NothingProposed);
    };
    let Some(track) = db::music::find_track_owned(pool, p.track_id, user_id).await? else {
        return Ok(AcceptOutcome::NothingProposed);
    };
    let keep = p.keep_album;
    db::music::apply_musicbrainz_metadata(
        pool,
        track.id,
        user_id,
        &proposed.title,
        proposed.artist.as_deref().or(track.artist.as_deref()),
        if keep { track.album.as_deref() } else { proposed.album.as_deref().or(track.album.as_deref()) },
        track.genre.as_deref(),
        if keep { track.track_number } else { proposed.track_number.or(track.track_number) },
        &proposed.mb_recording_id,
        if keep { None } else { proposed.mb_release_id.as_deref() },
        proposed.mb_artist_id.as_deref(),
        if keep { None } else { proposed.album_artist.as_deref() },
        if keep { None } else { proposed.mb_release_group_id.as_deref() },
        Some(p.run_id),
        None,
    )
    .await?;
    if !keep && proposed.disc_number.is_some() {
        db::music::set_disc(pool, track.id, proposed.disc_number, None).await?;
    }
    db::identify::set_decision(pool, proposal_id, "accepted").await?;
    Ok(AcceptOutcome::Accepted)
}

/// Undo every accept of a run that still stands; the proposals become pending
/// again. Returns how many tracks went back.
pub async fn revert_run(pool: &PgPool, run: &db::identify::Run) -> anyhow::Result<usize> {
    let mut n = 0;
    // One batch: the history shows the run's undo as one change.
    let edit = db::edits::Edit::new(run.user_id, "revert");
    for track_id in db::identify::identified_by_run(pool, run.id).await? {
        if db::music::revert_identity(pool, track_id, run.user_id, &edit).await? == db::music::RevertOutcome::Reverted {
            n += 1;
        }
    }
    sqlx::query(
        "UPDATE identify_proposals SET decision = 'pending', decided_at = NULL
          WHERE run_id = $1 AND decision = 'accepted'",
    )
    .bind(run.id)
    .execute(pool)
    .await?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, album: Option<&str>, folder: Option<&str>, album_id: Option<u128>) -> db::identify::ScopeTrack {
        db::identify::ScopeTrack {
            id: Uuid::new_v4(),
            title: title.into(),
            artist: Some("Artist".into()),
            album: album.map(Into::into),
            album_artist: None,
            track_number: None,
            disc_number: None,
            duration_secs: Some(200),
            album_id: album_id.map(Uuid::from_u128),
            folder: folder.map(Into::into),
        }
    }

    #[test]
    fn album_names_compare_without_discs_notes_and_case() {
        assert_eq!(album_norm("Alchemy - Disc One"), "alchemy");
        assert_eq!(album_norm("Alchemy: Dire Straits Live (Disc Two)"), "alchemy dire straits live");
        assert_eq!(album_norm("Alchemy (Disc 2) [FLAC]"), "alchemy");
        assert_eq!(album_norm("ALCHEMY  CD1"), "alchemy");
        assert_eq!(album_norm("Back to Black (Deluxe Edition)"), "back to black");
        assert_eq!(folder_norm("f/Dire Straits/Alchemy/CD 2"), "f/Dire Straits/Alchemy");
        assert_eq!(folder_norm("f/Dire Straits/Alchemy/Disc_1"), "f/Dire Straits/Alchemy");
        assert_eq!(folder_norm("f/Dire Straits/Alchemy"), "f/Dire Straits/Alchemy");
    }

    #[test]
    fn a_folder_split_across_album_entities_is_one_cluster() {
        // Two songs tagged with a release id, one without: three album entities' worth of one album.
        let songs = [
            song("Alles aus Liebe", Some("Alles aus Liebe"), Some("f/Singles/1993"), Some(1)),
            song("Auf dem Weg", Some("Alles aus Liebe"), Some("f/Singles/1993"), Some(1)),
            song("5 Minuten", Some("alles aus liebe"), Some("f/Singles/1993"), Some(2)),
        ];
        let groups = cluster(&songs);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "album");
        assert_eq!(groups[0].2.len(), 3);
    }

    #[test]
    fn discs_in_their_own_folders_are_one_album_and_untagged_songs_join_it() {
        let songs = [
            song("One", Some("Alchemy (Disc 1)"), Some("f/Alchemy/CD1"), Some(1)),
            song("Two", Some("Alchemy (Disc 2)"), Some("f/Alchemy/CD2"), Some(2)),
            song("Bonus", None, Some("f/Alchemy"), None),
        ];
        let groups = cluster(&songs);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].2.len(), 3);
    }

    #[test]
    fn uploads_stay_by_album_entity_and_lone_songs_are_looked_up_alone() {
        let songs = [
            song("A", Some("Same Name"), None, Some(1)),
            song("B", Some("Same Name"), None, Some(1)),
            song("C", Some("Same Name"), None, Some(2)),
            song("Lonely", Some("Other"), Some("f/x"), Some(3)),
            song("No album", None, None, None),
        ];
        let groups = cluster(&songs);
        let kinds: Vec<(&str, usize)> = groups.iter().map(|g| (g.0, g.2.len())).collect();
        // Two copies of "Same Name" (two entities) never merge; one of them is a single song.
        assert_eq!(kinds.iter().filter(|(k, n)| *k == "album" && *n == 2).count(), 1);
        assert_eq!(kinds.iter().filter(|(k, _)| *k == "track").count(), 3, "{kinds:?}");
    }

    #[test]
    fn backoff_doubles_to_five_minutes() {
        assert_eq!(backoff(1), Duration::from_secs(5));
        assert_eq!(backoff(2), Duration::from_secs(10));
        assert_eq!(backoff(6), Duration::from_secs(160));
        assert_eq!(backoff(7), Duration::from_secs(300));
        assert_eq!(backoff(10), Duration::from_secs(300));
    }

    #[test]
    fn the_most_common_credit_names_the_album() {
        let v = [Some("Queen"), None, Some("Queen"), Some("Queen & Bowie")];
        assert_eq!(most_common(v.into_iter()).as_deref(), Some("Queen"));
        assert_eq!(most_common([None, Some(" ")].into_iter()), None);
    }
}
