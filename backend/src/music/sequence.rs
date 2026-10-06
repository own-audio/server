// SPDX-License-Identifier: AGPL-3.0-or-later
//! Turning a set of candidate tracks into an ordered playlist.
//!
//! See `docs/music-signals-and-smart-playlists-plan.md` §4.3. This is the step
//! that makes the difference between a usable playlist and a random selection
//! in the right tempo range, and it is the reason the feature does not need a
//! model to be good: once tracks carry numbers, shaping them is deterministic
//! code.
//!
//! Three things happen here that filtering cannot do:
//!
//! 1. **No two consecutive tracks by one artist, and a cap per artist.** A
//!    family library is small enough that one prolific artist otherwise fills
//!    it, and the result reads as an album on shuffle.
//! 2. **An energy arc, not a flat band.** A two-hour Pilates set is roughly ten
//!    minutes warming up, ninety steady and twenty coming down — a flat filter
//!    gives none of that.
//! 3. **Weighted random, not top-N.** Deterministic ordering returns the same
//!    playlist every time and is stale within a week.

use crate::music::rules::{Arc, Limit, Rule, Sort};
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use uuid::Uuid;

/// One track as the sequencer sees it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub track_id: Uuid,
    pub artist: String,
    pub duration_secs: i32,
    /// `None` when the track has never been measured — it still plays, it just
    /// cannot be placed on the arc.
    pub energy: Option<f32>,
    /// Affinity weight already multiplied by the recency multiplier.
    pub score: f32,
}

/// Where the arc wants the energy at this point, `0.0..=1.0` through the
/// playlist. `None` means the arc imposes nothing.
fn target_energy(arc: Arc, position: f32) -> Option<f32> {
    match arc {
        Arc::None => None,
        Arc::Build => Some(0.3 + 0.6 * position),
        Arc::Unwind => Some(0.8 - 0.6 * position),
        Arc::WarmupSustainCooldown => Some(match position {
            // Ten per cent warming up, seventy-five per cent steady, the rest
            // coming down. Not a physiological claim — a shape that sounds
            // like a session rather than a shuffle.
            p if p < 0.10 => 0.35 + (p / 0.10) * 0.25,
            p if p < 0.85 => 0.60,
            p => 0.60 - ((p - 0.85) / 0.15) * 0.30,
        }),
    }
}

/// Build the playlist.
///
/// `seed` makes this testable; production passes a random one, which is what
/// keeps a dynamic playlist from returning the same order twice.
pub fn build(candidates: &[Candidate], rule: &Rule, seed: u64) -> Vec<Uuid> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut rng = StdRng::seed_from_u64(seed);
    let mut remaining: Vec<&Candidate> = candidates.iter().collect();

    // Deterministic sorts do their ordering up front; weighted-random picks as
    // it goes, because each pick depends on what the constraints still allow.
    match rule.sort {
        Sort::Weight => remaining.sort_by(|a, b| b.score.total_cmp(&a.score)),
        Sort::Random => {
            let mut keys: Vec<(f32, &Candidate)> =
                remaining.iter().map(|c| (rng.random::<f32>(), *c)).collect();
            keys.sort_by(|a, b| a.0.total_cmp(&b.0));
            remaining = keys.into_iter().map(|(_, c)| c).collect();
        }
        // The resolver already ordered these in SQL, where the timestamps are.
        Sort::LeastRecentlyPlayed => {}
        Sort::WeightedRandom => {}
    }

    let (max_tracks, max_secs) = match rule.limit {
        Limit::Count { n } => (n, i64::MAX),
        Limit::Duration { minutes } => (crate::music::rules::MAX_TRACKS, minutes as i64 * 60),
    };

    let mut picked: Vec<Uuid> = Vec::new();
    let mut per_artist: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut last_artist: Option<String> = None;
    let mut total_secs: i64 = 0;

    while picked.len() < max_tracks && !remaining.is_empty() {
        let progress = match rule.limit {
            Limit::Count { n } => picked.len() as f32 / n.max(1) as f32,
            Limit::Duration { .. } => {
                if max_secs > 0 {
                    total_secs as f32 / max_secs as f32
                } else {
                    0.0
                }
            }
        };
        let target = target_energy(rule.arc, progress.clamp(0.0, 1.0));
        let secs_left = max_secs.saturating_sub(total_secs);

        let eligible: Vec<usize> = remaining
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                if rule.sequence.no_consecutive_artist
                    && last_artist.as_deref() == Some(c.artist.as_str())
                    && !c.artist.is_empty()
                {
                    return false;
                }
                if !c.artist.is_empty()
                    && per_artist.get(&c.artist).copied().unwrap_or(0) >= rule.sequence.max_per_artist
                {
                    return false;
                }
                // A duration target is a budget, not a suggestion: never
                // overshoot. The last slot is filled by whatever still fits,
                // which is what lands a two-hour request within one track.
                if matches!(rule.limit, Limit::Duration { .. })
                    && c.duration_secs as i64 > secs_left
                {
                    return false;
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        if eligible.is_empty() {
            break;
        }

        let chosen = match rule.sort {
            Sort::WeightedRandom => {
                weighted_pick(&eligible, &remaining, target, &mut rng)
            }
            // The deterministic sorts keep their order; the constraints only
            // skip over what they forbid.
            _ => eligible[0],
        };

        let c = remaining.remove(chosen);
        total_secs += c.duration_secs.max(0) as i64;
        if !c.artist.is_empty() {
            *per_artist.entry(c.artist.clone()).or_insert(0) += 1;
            last_artist = Some(c.artist.clone());
        } else {
            last_artist = None;
        }
        picked.push(c.track_id);
    }

    picked
}

/// Pick one index, proportional to score and to how well the track fits the
/// arc.
///
/// A track off the arc is not excluded, only made less likely — excluding it
/// would empty the pool on a library that does not span the whole energy range,
/// which is the common case.
fn weighted_pick(
    eligible: &[usize],
    remaining: &[&Candidate],
    target: Option<f32>,
    rng: &mut StdRng,
) -> usize {
    let weights: Vec<f32> = eligible
        .iter()
        .map(|&i| {
            let c = remaining[i];
            let fit = match (target, c.energy) {
                (Some(t), Some(e)) => 1.0 - (e - t).abs(),
                // Unmeasured tracks sit at the middle rather than dropping out.
                (Some(_), None) => 0.5,
                (None, _) => 1.0,
            };
            (c.score.max(0.0) * fit.max(0.05)).max(f32::MIN_POSITIVE)
        })
        .collect();

    let total: f32 = weights.iter().sum();
    let mut roll = rng.random::<f32>() * total;
    for (n, &i) in eligible.iter().enumerate() {
        roll -= weights[n];
        if roll <= 0.0 {
            return i;
        }
    }
    eligible[eligible.len() - 1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::rules::Sequence;

    fn cand(artist: &str, secs: i32, energy: f32, score: f32) -> Candidate {
        Candidate {
            track_id: Uuid::new_v4(),
            artist: artist.to_string(),
            duration_secs: secs,
            energy: Some(energy),
            score,
        }
    }

    fn artists_of(ids: &[Uuid], pool: &[Candidate]) -> Vec<String> {
        ids.iter()
            .map(|id| {
                pool.iter()
                    .find(|c| c.track_id == *id)
                    .unwrap()
                    .artist
                    .clone()
            })
            .collect()
    }

    #[test]
    fn an_empty_pool_gives_an_empty_playlist() {
        assert!(build(&[], &Rule::default(), 1).is_empty());
    }

    /// The rule that keeps a playlist from reading as an album on shuffle.
    #[test]
    fn one_artist_never_appears_twice_in_a_row() {
        let pool: Vec<Candidate> = (0..12)
            .map(|i| cand(if i % 3 == 0 { "Queen" } else { "Other" }, 200, 0.5, 1.0))
            .collect();
        let rule = Rule {
            sequence: Sequence {
                max_per_artist: 10,
                no_consecutive_artist: true,
            },
            ..Default::default()
        };
        for seed in 0..25 {
            let names = artists_of(&build(&pool, &rule, seed), &pool);
            for w in names.windows(2) {
                assert_ne!(w[0], w[1], "consecutive {:?} at seed {seed}", w[0]);
            }
        }
    }

    #[test]
    fn the_per_artist_cap_is_respected() {
        let pool: Vec<Candidate> = (0..30)
            .map(|i| cand(&format!("A{}", i % 3), 200, 0.5, 1.0))
            .collect();
        let rule = Rule {
            sequence: Sequence {
                max_per_artist: 2,
                no_consecutive_artist: true,
            },
            ..Default::default()
        };
        let names = artists_of(&build(&pool, &rule, 7), &pool);
        for a in ["A0", "A1", "A2"] {
            assert!(
                names.iter().filter(|n| *n == a).count() <= 2,
                "{a} appeared more than twice in {names:?}"
            );
        }
    }

    /// A duration target is a budget. Overshooting a two-hour request by a
    /// track is what makes the feature feel careless.
    #[test]
    fn a_duration_target_is_never_overshot() {
        let pool: Vec<Candidate> = (0..80_i32)
            .map(|i| cand(&format!("A{i}"), 200 + (i % 5) * 30, 0.5, 1.0))
            .collect();
        let rule = Rule {
            limit: Limit::Duration { minutes: 30 },
            ..Default::default()
        };
        for seed in 0..15 {
            let ids = build(&pool, &rule, seed);
            let total: i64 = ids
                .iter()
                .map(|id| {
                    pool.iter().find(|c| c.track_id == *id).unwrap().duration_secs as i64
                })
                .sum();
            assert!(total <= 1800, "overshot at seed {seed}: {total}s");
            // And it should actually get close, not stop early.
            assert!(total > 1500, "stopped short at seed {seed}: {total}s");
        }
    }

    /// The arc is the difference between a session and a shuffle: the wind-down
    /// must actually be quieter than the middle.
    #[test]
    fn a_cooldown_arc_ends_quieter_than_it_peaks() {
        let pool: Vec<Candidate> = (0..120)
            .map(|i| cand(&format!("A{i}"), 200, (i % 10) as f32 / 10.0, 1.0))
            .collect();
        let rule = Rule {
            limit: Limit::Count { n: 40 },
            arc: Arc::WarmupSustainCooldown,
            sequence: Sequence {
                max_per_artist: 1,
                no_consecutive_artist: false,
            },
            ..Default::default()
        };
        let ids = build(&pool, &rule, 3);
        let energy_at = |id: &Uuid| pool.iter().find(|c| c.track_id == *id).unwrap().energy.unwrap();
        let mid: f32 = ids[10..30].iter().map(energy_at).sum::<f32>() / 20.0;
        let end: f32 = ids[36..].iter().map(energy_at).sum::<f32>() / (ids.len() - 36) as f32;
        assert!(end < mid, "cooldown {end} not below sustain {mid}");
    }

    #[test]
    fn build_rises_and_unwind_falls() {
        let pool: Vec<Candidate> = (0..120)
            .map(|i| cand(&format!("A{i}"), 200, (i % 10) as f32 / 10.0, 1.0))
            .collect();
        let seq = Sequence {
            max_per_artist: 1,
            no_consecutive_artist: false,
        };
        let energy_at = |id: &Uuid| pool.iter().find(|c| c.track_id == *id).unwrap().energy.unwrap();

        for (arc, rising) in [(Arc::Build, true), (Arc::Unwind, false)] {
            let rule = Rule {
                limit: Limit::Count { n: 40 },
                arc,
                sequence: seq.clone(),
                ..Default::default()
            };
            let ids = build(&pool, &rule, 11);
            let first: f32 = ids[..10].iter().map(energy_at).sum::<f32>() / 10.0;
            let last: f32 = ids[30..].iter().map(energy_at).sum::<f32>() / 10.0;
            if rising {
                assert!(last > first, "build did not rise: {first} -> {last}");
            } else {
                assert!(last < first, "unwind did not fall: {first} -> {last}");
            }
        }
    }

    /// Two resolves of the same dynamic playlist must not return the same
    /// order, or it is a static playlist with extra steps.
    #[test]
    fn weighted_random_gives_a_different_order_each_time() {
        let pool: Vec<Candidate> = (0..40).map(|i| cand(&format!("A{i}"), 200, 0.5, 1.0)).collect();
        let rule = Rule {
            limit: Limit::Count { n: 20 },
            ..Default::default()
        };
        assert_ne!(build(&pool, &rule, 1), build(&pool, &rule, 2));
    }

    /// An unmeasured track still plays. Dropping it would empty most playlists
    /// while a library is still being measured.
    #[test]
    fn tracks_with_no_energy_still_get_picked() {
        let mut pool: Vec<Candidate> = (0..10).map(|i| cand(&format!("A{i}"), 200, 0.9, 1.0)).collect();
        for c in pool.iter_mut().take(5) {
            c.energy = None;
        }
        let rule = Rule {
            limit: Limit::Count { n: 10 },
            arc: Arc::Unwind,
            sequence: Sequence {
                max_per_artist: 1,
                no_consecutive_artist: false,
            },
            ..Default::default()
        };
        assert_eq!(build(&pool, &rule, 5).len(), 10);
    }
}
