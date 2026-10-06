// SPDX-License-Identifier: AGPL-3.0-or-later
//! The saved query behind a smart playlist.
//!
//! See `docs/music-signals-and-smart-playlists-plan.md` §4.2. A smart playlist
//! is **a rule, an ordering and a refresh policy** — not a list of track ids.
//!
//! The schema is deliberately small and closed. Anything not expressible here
//! is not expressible at all, and that is the property that makes it safe to
//! accept one of these from a language model (§5.1): the model emits a rule,
//! never a tracklist, and a rule that fails validation is rejected rather than
//! half-applied.

use serde::{Deserialize, Serialize};

/// Hard ceiling on what one resolve may return, whatever the rule asks for.
pub const MAX_TRACKS: usize = 500;
pub const MAX_MINUTES: u32 = 24 * 60;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default)]
    pub filters: Filters,
    #[serde(default)]
    pub limit: Limit,
    #[serde(default)]
    pub arc: Arc,
    #[serde(default)]
    pub sequence: Sequence,
    #[serde(default)]
    pub sort: Sort,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Filters {
    /// Inclusive `[min, max]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<[f32; 2]>,
    /// Inclusive `[min, max]`, each in `[0, 1]`. See migration 0072 — the
    /// number means nothing on its own, only relative to this library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genres: Option<StringSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artists: Option<StringSet>,
    // NOTE: the plan (§4.2) also lists a `year` filter. `music_tracks` has no
    // year column — not in the tags we read, not from MusicBrainz — so it is
    // deliberately absent here rather than present and failing at query time.
    // Adding it means a column and a backfill first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starred: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_rating: Option<i16>,
    /// Not played by me in this many days. The core of "resurface what the
    /// library already holds".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_played_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_days: Option<i32>,
    /// Played by other family members but never by me — the one cross-member
    /// signal that needs no model at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played_by_others_not_me: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StringSet {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Limit {
    Count { n: usize },
    Duration { minutes: u32 },
}

impl Default for Limit {
    fn default() -> Self {
        Limit::Count { n: 50 }
    }
}

/// The energy shape over the playlist.
///
/// A two-hour Pilates set is not two hours of one intensity: roughly ten
/// minutes warming up, ninety steady, twenty coming down. This is what makes
/// the difference between a usable playlist and a random selection in the right
/// tempo range — and once tracks carry numbers it is ordinary deterministic
/// code.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Arc {
    /// No shaping — order by score and the sequencing constraints alone.
    #[default]
    None,
    WarmupSustainCooldown,
    /// Rising throughout.
    Build,
    /// Falling throughout: an evening, a wind-down.
    Unwind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    pub max_per_artist: u32,
    pub no_consecutive_artist: bool,
}

impl Default for Sequence {
    fn default() -> Self {
        // Two per artist is the difference between a playlist and an album on
        // shuffle. The rule exists because a family library is small enough
        // that one prolific artist can otherwise fill it.
        Sequence {
            max_per_artist: 2,
            no_consecutive_artist: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    /// Weighted random, **the default on purpose**: a deterministic top-N
    /// returns the same playlist every time and is stale within a week.
    #[default]
    WeightedRandom,
    /// Highest affinity first. Useful for a short "best of", bad for rotation.
    Weight,
    /// Longest unplayed first.
    LeastRecentlyPlayed,
    Random,
}

/// Why a rule was rejected. Returned to the caller as a 400, and — when the
/// rule came from a language model — as the reason to re-ask rather than to
/// guess.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RuleError {
    #[error("{field} range is inverted: min {min} is above max {max}")]
    InvertedRange { field: &'static str, min: f32, max: f32 },
    #[error("{field} value {value} is outside {lo}..={hi}")]
    OutOfRange {
        field: &'static str,
        value: f32,
        lo: f32,
        hi: f32,
    },
    #[error("a playlist of {0} tracks is beyond the {MAX_TRACKS} limit")]
    TooManyTracks(usize),
    #[error("a playlist of {0} minutes is beyond the {MAX_MINUTES} limit")]
    TooLong(u32),
    #[error("max_per_artist must be at least 1")]
    NoRoomForAnyArtist,
    #[error("the same genre is both included and excluded: {0}")]
    ContradictoryGenre(String),
}

impl Rule {
    /// Reject anything that cannot mean what it says.
    ///
    /// Validation is strict because this is the trust boundary for
    /// model-generated rules: a rule that survives here is one the resolver can
    /// run without further checking, and one that does not is a clean refusal
    /// rather than a playlist that quietly means something else.
    pub fn validate(&self) -> Result<(), RuleError> {
        if let Some([lo, hi]) = self.filters.bpm {
            check_range("bpm", lo, hi)?;
            check_bounds("bpm", lo, 0.0, 400.0)?;
            check_bounds("bpm", hi, 0.0, 400.0)?;
        }
        if let Some([lo, hi]) = self.filters.energy {
            check_range("energy", lo, hi)?;
            check_bounds("energy", lo, 0.0, 1.0)?;
            check_bounds("energy", hi, 0.0, 1.0)?;
        }
        if let Some(r) = self.filters.min_rating {
            check_bounds("min_rating", r as f32, 1.0, 5.0)?;
        }
        if let Some(d) = self.filters.not_played_days {
            check_bounds("not_played_days", d as f32, 0.0, 36_500.0)?;
        }
        if let Some(d) = self.filters.added_days {
            check_bounds("added_days", d as f32, 0.0, 36_500.0)?;
        }

        match self.limit {
            Limit::Count { n } if n == 0 || n > MAX_TRACKS => {
                return Err(RuleError::TooManyTracks(n));
            }
            Limit::Duration { minutes } if minutes == 0 || minutes > MAX_MINUTES => {
                return Err(RuleError::TooLong(minutes));
            }
            _ => {}
        }

        if self.sequence.max_per_artist == 0 {
            return Err(RuleError::NoRoomForAnyArtist);
        }

        // A model asked for "rock but not rock" produces this, and silently
        // returning nothing would look like an empty library rather than a
        // contradictory request.
        if let Some(g) = &self.filters.genres {
            for inc in &g.include {
                if g.exclude.iter().any(|e| e.eq_ignore_ascii_case(inc)) {
                    return Err(RuleError::ContradictoryGenre(inc.clone()));
                }
            }
        }

        Ok(())
    }
}

fn check_range(field: &'static str, min: f32, max: f32) -> Result<(), RuleError> {
    if min > max {
        Err(RuleError::InvertedRange { field, min, max })
    } else {
        Ok(())
    }
}

fn check_bounds(field: &'static str, value: f32, lo: f32, hi: f32) -> Result<(), RuleError> {
    if value < lo || value > hi {
        Err(RuleError::OutOfRange {
            field,
            value,
            lo,
            hi,
        })
    } else {
        Ok(())
    }
}

/// The five presets worth shipping.
///
/// Every one answers the premise in §1 — a family library's problem is not
/// *recommend something unknown* but **resurface something forgotten** — and
/// four of the five need no acoustic data at all, so they work before any
/// measurement has happened.
pub fn presets() -> Vec<(&'static str, &'static str, Rule)> {
    vec![
        (
            "forgotten-favourites",
            "Forgotten favourites",
            Rule {
                filters: Filters {
                    starred: Some(true),
                    not_played_days: Some(365),
                    ..Default::default()
                },
                limit: Limit::Count { n: 50 },
                ..Default::default()
            },
        ),
        (
            "new-in-the-library",
            "New in the library",
            Rule {
                filters: Filters {
                    added_days: Some(30),
                    not_played_days: Some(36_500),
                    ..Default::default()
                },
                limit: Limit::Count { n: 50 },
                sort: Sort::LeastRecentlyPlayed,
                ..Default::default()
            },
        ),
        (
            "back-in-rotation",
            "Back in rotation",
            Rule {
                filters: Filters {
                    min_rating: Some(4),
                    not_played_days: Some(60),
                    ..Default::default()
                },
                limit: Limit::Count { n: 50 },
                ..Default::default()
            },
        ),
        (
            "what-the-family-plays",
            "What the family plays",
            Rule {
                filters: Filters {
                    played_by_others_not_me: Some(true),
                    ..Default::default()
                },
                limit: Limit::Count { n: 50 },
                ..Default::default()
            },
        ),
        (
            "genre-station",
            "Genre station",
            Rule {
                limit: Limit::Count { n: 100 },
                ..Default::default()
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_rule_is_valid_and_means_the_whole_library() {
        let r = Rule::default();
        assert!(r.validate().is_ok());
        assert_eq!(r.limit, Limit::Count { n: 50 });
        assert_eq!(r.sort, Sort::WeightedRandom);
    }

    /// The default matters: a deterministic top-N returns the same playlist
    /// every time and is stale within a week.
    #[test]
    fn the_default_sort_is_weighted_random() {
        assert_eq!(Sort::default(), Sort::WeightedRandom);
    }

    #[test]
    fn an_inverted_range_is_refused() {
        let r = Rule {
            filters: Filters {
                bpm: Some([140.0, 90.0]),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            r.validate(),
            Err(RuleError::InvertedRange { field: "bpm", .. })
        ));
    }

    #[test]
    fn energy_outside_zero_to_one_is_refused() {
        let r = Rule {
            filters: Filters {
                energy: Some([0.2, 3.0]),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            r.validate(),
            Err(RuleError::OutOfRange { field: "energy", .. })
        ));
    }

    /// A model asked for "rock but not rock" writes this. Returning nothing
    /// would look like an empty library rather than a contradictory request.
    #[test]
    fn a_genre_both_included_and_excluded_is_refused() {
        let r = Rule {
            filters: Filters {
                genres: Some(StringSet {
                    include: vec!["Rock".into()],
                    exclude: vec!["rock".into()],
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(r.validate(), Err(RuleError::ContradictoryGenre(_))));
    }

    #[test]
    fn absurd_limits_are_refused() {
        let count = Rule {
            limit: Limit::Count { n: 10_000 },
            ..Default::default()
        };
        assert!(matches!(count.validate(), Err(RuleError::TooManyTracks(_))));

        let zero = Rule {
            limit: Limit::Count { n: 0 },
            ..Default::default()
        };
        assert!(matches!(zero.validate(), Err(RuleError::TooManyTracks(0))));

        let long = Rule {
            limit: Limit::Duration { minutes: 10_000 },
            ..Default::default()
        };
        assert!(matches!(long.validate(), Err(RuleError::TooLong(_))));
    }

    #[test]
    fn a_rule_with_no_room_for_any_artist_is_refused() {
        let r = Rule {
            sequence: Sequence {
                max_per_artist: 0,
                no_consecutive_artist: true,
            },
            ..Default::default()
        };
        assert_eq!(r.validate(), Err(RuleError::NoRoomForAnyArtist));
    }

    /// The closed schema is the property that makes a model-generated rule safe
    /// to run: anything the model invents is rejected at the door, not
    /// half-applied.
    #[test]
    fn an_unknown_field_is_rejected_at_parse_time() {
        let json = r#"{"filters":{"bpm":[90,120],"vibe":"chill"}}"#;
        assert!(serde_json::from_str::<Rule>(json).is_err());
    }

    #[test]
    fn a_rule_survives_a_round_trip_through_json() {
        let r = Rule {
            filters: Filters {
                bpm: Some([95.0, 125.0]),
                energy: Some([0.4, 0.7]),
                ..Default::default()
            },
            limit: Limit::Duration { minutes: 120 },
            arc: Arc::WarmupSustainCooldown,
            ..Default::default()
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Rule>(&json).unwrap(), r);
    }

    #[test]
    fn every_preset_is_valid() {
        for (slug, _, rule) in presets() {
            assert!(rule.validate().is_ok(), "preset {slug} does not validate");
        }
    }
}
