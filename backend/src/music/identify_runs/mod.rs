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

/// Split the scope into groups: the tracks of one album entity together, every
/// track without an album tag on its own.
async fn plan_groups(pool: &PgPool, run: &db::identify::Run) -> anyhow::Result<()> {
    let tracks = db::identify::scope_tracks(pool, run).await?;
    // An album with one track in scope is looked up as that track: an album
    // search on one title finds any record that holds it.
    let mut album_sizes: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    for t in &tracks {
        if let Some(a) = t.album_id {
            *album_sizes.entry(a).or_default() += 1;
        }
    }
    let mut albums: BTreeMap<Uuid, (String, Vec<Uuid>)> = BTreeMap::new();
    let mut groups: Vec<(&str, String, Vec<Uuid>)> = Vec::new();
    for t in &tracks {
        match (t.album_id, t.album.as_deref().filter(|a| !a.trim().is_empty())) {
            (Some(album_id), Some(album)) if album_sizes.get(&album_id).copied().unwrap_or(0) > 1 => {
                let entry = albums.entry(album_id).or_insert_with(|| {
                    let by = t.album_artist.as_deref().or(t.artist.as_deref()).unwrap_or("");
                    (if by.is_empty() { album.to_string() } else { format!("{album} — {by}") }, Vec::new())
                });
                entry.1.push(t.id);
            }
            _ => groups.push(("track", t.title.clone(), vec![t.id])),
        }
    }
    let mut album_groups: Vec<(&str, String, Vec<Uuid>)> =
        albums.into_values().map(|(label, ids)| ("album", label, ids)).collect();
    album_groups.sort_by_key(|g| g.1.to_lowercase());
    album_groups.extend(groups);
    db::identify::insert_groups(pool, run.id, &album_groups, tracks.len() as i32).await
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
    let album = files.first().and_then(|f| f.album.clone());
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
