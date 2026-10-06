// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-user, per-track preference weight.
//!
//! One number in `[0, 1]`, multiplied into every automatic selection. See
//! `docs/music-signals-and-smart-playlists-plan.md` §2.5.
//!
//! **None of these constants are defensible from first principles.** They are
//! starting values, to be tuned against one real library. They live together
//! here, and the raw counters they are derived from are stored in
//! `music_track_affinity`, so retuning is an UPDATE over that table rather than
//! a replay of the whole session log.

use chrono::{DateTime, Duration, Utc};

/// The one boundary that matters: above this fraction the track effectively
/// played, and moving on near the end is not a complaint. Below it, the user
/// did not want this now.
///
/// The plan describes three bands (early / partial / late). Two are stored,
/// because a skip at 10 % and a skip at 50 % are both "not this one" and
/// splitting them would mean a third column carrying no decision.
pub const LATE_SKIP_FRACTION: f64 = 0.70;

/// What one early skip multiplies the weight by, before recovery.
const EARLY_SKIP_PENALTY: f64 = 0.5;

/// A late skip still counts, but barely.
const LATE_SKIP_PENALTY: f64 = 0.9;

/// How long until a skip has lost half its effect. The user's own framing, and
/// the reason `dislike` and `banned` are separate controls: *a track that did
/// not suit the moment is not a track you never want to hear again.*
const SKIP_HALF_LIFE_DAYS: f64 = 28.0;

/// A dislike is a strong statement, so it recovers far more slowly than a skip.
const DISLIKE_PENALTY: f64 = 0.1;
const DISLIKE_HALF_LIFE_DAYS: f64 = 180.0;

/// Played recently, so rest it — otherwise rotation collapses onto the same
/// forty tracks. Deliberately NOT part of [`weight`]; see [`recency_multiplier`].
const RECENCY_WINDOW_DAYS: f64 = 14.0;
const RECENCY_FLOOR: f64 = 0.35;

/// How a track's explicit marks move its starting point.
const STARRED_BOOST: f64 = 1.3;
const HIGH_RATING_BOOST: f64 = 1.2;
const LOW_RATING_PENALTY: f64 = 0.6;

/// Everything known about one (user, track) pair at scoring time.
#[derive(Debug, Clone, Default)]
pub struct AffinityInputs {
    pub play_count: i32,
    pub early_skips: i32,
    pub late_skips: i32,
    pub last_played_at: Option<DateTime<Utc>>,
    pub last_skipped_at: Option<DateTime<Utc>>,
    pub starred: bool,
    /// 1–5, or None when unrated. 0 is not a rating — the protocol uses it to
    /// mean "unrate", stored as the absence of a row (0036).
    pub rating: Option<i16>,
    pub disliked_at: Option<DateTime<Utc>>,
}

/// Decay a penalty back toward 1.0 with the given half-life.
///
/// `penalty` is what the event costs at the moment it happens; the returned
/// multiplier moves from that value toward 1.0 as it ages. An event with no
/// timestamp is treated as fully recovered — we know it happened but not when,
/// and guessing "just now" would be the harsher of the two errors.
fn recovered(penalty: f64, at: Option<DateTime<Utc>>, now: DateTime<Utc>, half_life_days: f64) -> f64 {
    let Some(at) = at else { return 1.0 };
    let age_days = (now - at).max(Duration::zero()).num_seconds() as f64 / 86_400.0;
    let remaining = 0.5_f64.powf(age_days / half_life_days);
    penalty + (1.0 - penalty) * (1.0 - remaining)
}

/// How much this user likes this track, in `[0, 1]`.
///
/// Multiplicative rather than additive so that no single signal can dominate:
/// a starred track that is skipped early every time still falls, and a track
/// skipped once months ago is essentially back where it started.
///
/// **Taste only — nothing about whether it should play right now.** Rest goes
/// through [`recency_multiplier`], applied at selection time. Folding it in
/// here was tried and was wrong twice over: it made a track played this morning
/// score below a track skipped four times, and because this value is stored by
/// a periodic rollup, the rest penalty froze at whatever it was when the job
/// last ran instead of wearing off.
pub fn weight(inputs: &AffinityInputs, now: DateTime<Utc>) -> f64 {
    let mut base = 1.0_f64;
    if inputs.starred {
        base *= STARRED_BOOST;
    }
    match inputs.rating {
        Some(4..=5) => base *= HIGH_RATING_BOOST,
        Some(1..=2) => base *= LOW_RATING_PENALTY,
        _ => {}
    }

    // Repeated skips compound, but each one recovers. Applying the decay to the
    // aggregate rather than per-event is a deliberate simplification: we keep
    // counts, not a list of skip timestamps, and only the most recent skip's
    // age is known.
    let early = EARLY_SKIP_PENALTY.powi(inputs.early_skips.clamp(0, 8));
    let late = LATE_SKIP_PENALTY.powi(inputs.late_skips.clamp(0, 8));
    let skip_penalty = recovered(early * late, inputs.last_skipped_at, now, SKIP_HALF_LIFE_DAYS);

    let dislike_penalty = match inputs.disliked_at {
        Some(_) => recovered(
            DISLIKE_PENALTY,
            inputs.disliked_at,
            now,
            DISLIKE_HALF_LIFE_DAYS,
        ),
        None => 1.0,
    };

    // Completions push back up, with diminishing returns — a track played
    // fifty times is not fifty times more wanted than one played twice.
    let play_boost = 1.0 + (inputs.play_count.max(0) as f64).ln_1p() * 0.1;

    (base * play_boost * skip_penalty * dislike_penalty).clamp(0.0, 1.0)
}

/// How much to rest a track that was played recently, in `[RECENCY_FLOOR, 1]`.
///
/// Applied at selection time, against `music_track_affinity.last_played_at`,
/// never stored: it changes every hour on its own, and a stored copy would be
/// wrong the moment the rollup finished.
///
/// This is not a judgement about the track. A favourite played this morning is
/// still a favourite; it just should not come round again before lunch.
pub fn recency_multiplier(last_played_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> f64 {
    let Some(at) = last_played_at else { return 1.0 };
    let age_days = (now - at).max(Duration::zero()).num_seconds() as f64 / 86_400.0;
    if age_days >= RECENCY_WINDOW_DAYS {
        1.0
    } else {
        RECENCY_FLOOR + (1.0 - RECENCY_FLOOR) * (age_days / RECENCY_WINDOW_DAYS)
    }
}

/// Which bucket a skip falls into, from how much of the track was heard.
///
/// `None` when the track's duration is unknown or nonsensical — a skip we
/// cannot place is not counted, because guessing would put it in the harsher
/// bucket half the time.
pub fn skip_bucket(seconds_listened: i32, duration_secs: Option<i32>) -> Option<SkipKind> {
    let duration = duration_secs.filter(|d| *d > 0)? as f64;
    let fraction = seconds_listened.max(0) as f64 / duration;
    if fraction < LATE_SKIP_FRACTION {
        Some(SkipKind::Early)
    } else {
        Some(SkipKind::Late)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipKind {
    Early,
    Late,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-21T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn days_ago(d: i64) -> Option<DateTime<Utc>> {
        Some(now() - Duration::days(d))
    }

    #[test]
    fn an_untouched_track_sits_at_one() {
        assert_eq!(weight(&AffinityInputs::default(), now()), 1.0);
    }

    /// The core of the user's own framing: a skip should lower a track's odds
    /// without removing it, and it should fade. That is what the ban button is
    /// for, not this.
    #[test]
    fn a_skip_lowers_the_weight_but_never_to_zero() {
        let fresh = AffinityInputs {
            early_skips: 1,
            last_skipped_at: days_ago(0),
            ..Default::default()
        };
        let w = weight(&fresh, now());
        assert!(w < 1.0, "a fresh skip must lower the weight, got {w}");
        assert!(w > 0.0, "a skip must never zero a track out, got {w}");
    }

    #[test]
    fn a_skip_recovers_with_time() {
        let mk = |d: i64| AffinityInputs {
            early_skips: 1,
            last_skipped_at: days_ago(d),
            ..Default::default()
        };
        let fresh = weight(&mk(0), now());
        let month = weight(&mk(28), now());
        let year = weight(&mk(365), now());

        assert!(month > fresh, "a month-old skip must hurt less than a fresh one");
        assert!(year > month, "recovery must keep going");
        assert!(year > 0.97, "a year-old single skip is effectively forgotten, got {year}");
    }

    #[test]
    fn repeated_skips_compound() {
        let mk = |n: i32| AffinityInputs {
            early_skips: n,
            last_skipped_at: days_ago(0),
            ..Default::default()
        };
        assert!(weight(&mk(3), now()) < weight(&mk(1), now()));
    }

    /// A dislike is a much stronger statement than a skip, and must still be
    /// stronger a month later — otherwise the two controls mean the same thing.
    #[test]
    fn a_dislike_outweighs_a_skip_and_outlasts_it() {
        let skipped = AffinityInputs {
            early_skips: 1,
            last_skipped_at: days_ago(30),
            ..Default::default()
        };
        let disliked = AffinityInputs {
            disliked_at: days_ago(30),
            ..Default::default()
        };
        assert!(
            weight(&disliked, now()) < weight(&skipped, now()),
            "a dislike must still bite after a month"
        );
    }

    #[test]
    fn a_starred_track_outranks_a_plain_one_even_after_a_skip() {
        let plain = AffinityInputs::default();
        let starred_skipped = AffinityInputs {
            starred: true,
            early_skips: 1,
            last_skipped_at: days_ago(60),
            ..Default::default()
        };
        assert!(weight(&starred_skipped, now()) >= weight(&plain, now()) * 0.95);
    }

    /// Resting a track is not a judgement about it, so it must not touch the
    /// taste weight — only the selection-time multiplier.
    #[test]
    fn a_just_played_track_rests_then_returns() {
        assert!(recency_multiplier(days_ago(0), now()) < recency_multiplier(days_ago(7), now()));
        assert!(recency_multiplier(days_ago(7), now()) < recency_multiplier(days_ago(14), now()));
        assert_eq!(recency_multiplier(days_ago(30), now()), 1.0);
        assert_eq!(recency_multiplier(None, now()), 1.0);
    }

    /// Regression, found by running the rollup against real sessions: a track
    /// played four times scored 0.203 while a track skipped four times scored
    /// 0.273, because the rest penalty was being multiplied into the stored
    /// weight. Playing a track must never rank it below one you keep skipping.
    #[test]
    fn playing_a_track_ranks_it_above_one_you_keep_skipping() {
        let played = AffinityInputs {
            play_count: 4,
            early_skips: 1,
            last_played_at: days_ago(0),
            last_skipped_at: days_ago(0),
            ..Default::default()
        };
        let skipped = AffinityInputs {
            play_count: 0,
            early_skips: 3,
            late_skips: 1,
            last_skipped_at: days_ago(8),
            ..Default::default()
        };
        assert!(
            weight(&played, now()) > weight(&skipped, now()),
            "played {} must outrank skipped {}",
            weight(&played, now()),
            weight(&skipped, now())
        );
    }

    #[test]
    fn the_weight_never_leaves_its_range() {
        for stars in [false, true] {
            for rating in [None, Some(1), Some(5)] {
                for skips in [0, 1, 50] {
                    let w = weight(
                        &AffinityInputs {
                            play_count: 10_000,
                            early_skips: skips,
                            late_skips: skips,
                            last_skipped_at: days_ago(1),
                            last_played_at: days_ago(1),
                            starred: stars,
                            rating,
                            disliked_at: None,
                        },
                        now(),
                    );
                    assert!((0.0..=1.0).contains(&w), "out of range: {w}");
                }
            }
        }
    }

    /// A skip we cannot place must not be counted. Guessing would put it in the
    /// harsher bucket half the time, and the harsher bucket is the one that
    /// teaches the model something false.
    #[test]
    fn a_skip_with_no_duration_is_not_bucketed() {
        assert_eq!(skip_bucket(5, None), None);
        assert_eq!(skip_bucket(5, Some(0)), None);
        assert_eq!(skip_bucket(5, Some(-1)), None);
    }

    #[test]
    fn skips_bucket_by_position() {
        assert_eq!(skip_bucket(5, Some(240)), Some(SkipKind::Early));
        assert_eq!(skip_bucket(100, Some(240)), Some(SkipKind::Early));
        assert_eq!(skip_bucket(230, Some(240)), Some(SkipKind::Late));
    }
}
