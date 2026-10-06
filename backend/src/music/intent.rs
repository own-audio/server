// SPDX-License-Identifier: AGPL-3.0-or-later
//! Turning a sentence into a rule.
//!
//! See `docs/music-signals-and-smart-playlists-plan.md` §5. The target
//! interaction, in the user's words: *"I'm going for Pilates, make me a
//! two-hour playlist from my collection and start playing."*
//!
//! # The rule that matters
//!
//! **This produces a [`Rule`], never a tracklist.** Selection and sequencing
//! stay in [`crate::music::sequence`], where they are deterministic and
//! inspectable. A model asked for tracks invents tracks the library does not
//! have; twenty thousand tracks do not fit in a context window; and when a
//! playlist comes out wrong, a rule can be read and a hallucinated tracklist
//! cannot.
//!
//! # What this implementation is, honestly
//!
//! A **deterministic baseline**, not a language model. It recognises a small,
//! documented set of patterns — a duration, an activity, a mood, a few library
//! qualifiers — and returns [`IntentError::NotUnderstood`] for everything else.
//! It ships with no dependency, no per-request cost and no network, and it is
//! wrong in a way that is obvious rather than plausible.
//!
//! The Apple clients are expected to do better than this locally, with
//! `FoundationModels` and guided generation (`MUSIC_AI_STUDY.md` §2). This
//! endpoint exists so Android, Windows, tvOS and the web reach the same
//! feature, and so the *contract* — text in, validated rule out — is fixed
//! before any model is chosen. Swapping a model in behind it changes nothing
//! downstream.
//!
//! Nothing about the library is sent anywhere by this code. It reads a
//! sentence and the caller's vocabulary of genres; it never sees track titles
//! or listening history.

use crate::music::rules::{Arc, Limit, Rule, Sort, StringSet};

/// Why a sentence produced no rule.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum IntentError {
    #[error("nothing in that names music I can look for")]
    NotUnderstood,
}

/// An activity or mood, and the shape it implies.
struct Profile {
    keywords: &'static [&'static str],
    bpm: Option<[f32; 2]>,
    energy: Option<[f32; 2]>,
    arc: Arc,
}

/// Deliberately short. Each entry is a claim about what someone means, and a
/// wrong one is worse than a refusal — it returns a confident playlist for a
/// request it misread.
const PROFILES: &[Profile] = &[
    Profile {
        // Sustained moderate effort with a warm-up and a wind-down.
        keywords: &["pilates", "yoga", "stretch", "jóga", "strečink"],
        bpm: Some([95.0, 125.0]),
        energy: Some([0.35, 0.7]),
        arc: Arc::WarmupSustainCooldown,
    },
    Profile {
        keywords: &["run", "running", "workout", "gym", "cardio", "běh", "posilovna"],
        bpm: Some([140.0, 180.0]),
        energy: Some([0.6, 1.0]),
        arc: Arc::Build,
    },
    Profile {
        keywords: &["party", "dance", "dancing", "večírek", "tancovat"],
        bpm: Some([110.0, 140.0]),
        energy: Some([0.65, 1.0]),
        arc: Arc::Build,
    },
    Profile {
        keywords: &["sleep", "relax", "calm", "chill", "wind down", "spát", "klid"],
        bpm: None,
        energy: Some([0.0, 0.35]),
        arc: Arc::Unwind,
    },
    Profile {
        keywords: &["focus", "study", "work", "reading", "soustředění", "učení"],
        bpm: None,
        energy: Some([0.2, 0.5]),
        arc: Arc::None,
    },
    Profile {
        keywords: &["dinner", "evening", "večeře", "večer"],
        bpm: None,
        energy: Some([0.15, 0.5]),
        arc: Arc::Unwind,
    },
];

/// Parse a request into a rule.
///
/// `genres` is the caller's own genre vocabulary, so "put on some jazz" can
/// match what the library actually calls jazz rather than a guess.
pub fn parse(text: &str, genres: &[String]) -> Result<Rule, IntentError> {
    let lower = text.to_lowercase();
    let mut rule = Rule::default();
    let mut understood = false;

    // Computed before the activity profile below, so a profile can tell
    // whether its own trigger word doubles as one of these — a real bug,
    // found live: "dance disco for two hours" matched the party profile via
    // "dance" (bpm 110-140, energy 0.65-1.0) *and* a genre literally named
    // "Dance" via the same word, and the two ANDed together down to zero
    // tracks — a perfectly reasonable request, refused for a reason no user
    // could see.
    let matched_genres: Vec<String> = genres
        .iter()
        .filter(|g| !g.trim().is_empty() && lower.contains(&g.to_lowercase()))
        .cloned()
        .collect();
    let matched_genres_lower: Vec<String> =
        matched_genres.iter().map(|g| g.to_lowercase()).collect();

    if let Some(p) = PROFILES
        .iter()
        .find(|p| p.keywords.iter().any(|k| lower.contains(k)))
    {
        // Which keyword actually fired, so it can be checked against the
        // genres already matched above.
        let trigger = p.keywords.iter().find(|k| lower.contains(*k));
        let overlaps_a_genre = trigger.is_some_and(|k| {
            matched_genres_lower
                .iter()
                .any(|g| g.contains(k) || k.contains(g.as_str()))
        });

        // A genre match is ground truth for this specific library; a
        // profile's bpm/energy band is a generic guess about what "party" or
        // "pilates" usually sounds like, and this library's own tracks are
        // not obligated to agree with it. When the same word triggers both,
        // trust the concrete match and drop the guess rather than AND them
        // together. The arc still applies — it only shapes sequencing, it
        // cannot empty a result the way a filter can.
        if !overlaps_a_genre {
            rule.filters.bpm = p.bpm;
            rule.filters.energy = p.energy;
        }
        rule.arc = p.arc;
        understood = true;
    }

    if let Some(minutes) = parse_minutes(&lower) {
        rule.limit = Limit::Duration { minutes };
        understood = true;
    }

    if !matched_genres.is_empty() {
        rule.filters.genres = Some(StringSet {
            include: matched_genres,
            exclude: Vec::new(),
        });
        understood = true;
    }

    // Library qualifiers — the "resurface what I already have" half of the
    // feature, which needs no acoustic data at all.
    if contains_any(&lower, &["forgotten", "haven't heard", "zapomenut", "dlouho jsem"]) {
        rule.filters.not_played_days = Some(365);
        rule.filters.starred = Some(true);
        understood = true;
    }
    if contains_any(&lower, &["new", "recently added", "nov", "přidan"]) {
        rule.filters.added_days = Some(30);
        understood = true;
    }
    if contains_any(&lower, &["favourite", "favorite", "loved", "oblíben"]) {
        rule.filters.starred = Some(true);
        understood = true;
    }
    if contains_any(&lower, &["never played", "nikdy", "neslyšel"]) {
        rule.filters.not_played_days = Some(36_500);
        understood = true;
    }

    if !understood {
        return Err(IntentError::NotUnderstood);
    }

    // A request with no explicit length is a listening session, not a
    // two-track answer.
    if rule.limit == Limit::default() && rule.arc != Arc::None {
        rule.limit = Limit::Duration { minutes: 60 };
    }
    rule.sort = Sort::WeightedRandom;

    // Belt and braces: the parser is supposed to produce only valid rules, but
    // this is the trust boundary a model will later sit behind, and a rule that
    // escapes validation is exactly what must never happen.
    rule.validate().map_err(|_| IntentError::NotUnderstood)?;
    Ok(rule)
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Pull a duration out of the sentence.
///
/// Handles "2 hours", "90 minutes", "an hour and a half", and the Czech forms,
/// because that is what the person asking actually types.
fn parse_minutes(lower: &str) -> Option<u32> {
    if contains_any(lower, &["hour and a half", "hodinu a půl", "půldruhé hodiny"]) {
        return Some(90);
    }
    if contains_any(lower, &["half an hour", "půl hodiny"]) {
        return Some(30);
    }

    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    for (i, w) in words.iter().enumerate() {
        let Ok(n) = w.parse::<u32>() else { continue };
        // Look at the next two words: "2 hours", "90 min", "2 hodiny".
        let unit = words
            .get(i + 1)
            .into_iter()
            .chain(words.get(i + 2))
            .find(|u| !u.is_empty());
        let Some(unit) = unit else { continue };

        if unit.starts_with("hour") || unit.starts_with("hod") {
            return Some((n * 60).min(crate::music::rules::MAX_MINUTES));
        }
        if unit.starts_with("min") {
            return Some(n.clamp(1, crate::music::rules::MAX_MINUTES));
        }
    }

    // "an hour" with no number.
    if contains_any(lower, &["an hour", "one hour", "hodinu", "hodina"]) {
        return Some(60);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genres() -> Vec<String> {
        vec!["Rock".into(), "Jazz".into(), "Electronic".into()]
    }

    /// The sentence this whole feature was designed around.
    #[test]
    fn the_pilates_request_becomes_a_shaped_two_hour_rule() {
        let r = parse(
            "I'm going for Pilates, make me a two hour playlist from my collection",
            &genres(),
        );
        // "two" is a word, not a digit — the duration comes from the profile
        // default, and the shape is what matters.
        let r = r.expect("should be understood");
        assert_eq!(r.arc, Arc::WarmupSustainCooldown);
        assert_eq!(r.filters.bpm, Some([95.0, 125.0]));
        assert!(r.validate().is_ok());
    }

    #[test]
    fn a_numeric_duration_is_taken() {
        let r = parse("pilates for 120 minutes", &genres()).unwrap();
        assert_eq!(r.limit, Limit::Duration { minutes: 120 });

        let r = parse("2 hours of running music", &genres()).unwrap();
        assert_eq!(r.limit, Limit::Duration { minutes: 120 });

        let r = parse("jedu na pilates na 2 hodiny", &genres()).unwrap();
        assert_eq!(r.limit, Limit::Duration { minutes: 120 });
    }

    #[test]
    fn half_hours_are_understood() {
        assert_eq!(
            parse("relax for an hour and a half", &genres()).unwrap().limit,
            Limit::Duration { minutes: 90 }
        );
        assert_eq!(
            parse("chill for half an hour", &genres()).unwrap().limit,
            Limit::Duration { minutes: 30 }
        );
    }

    #[test]
    fn a_workout_builds_and_a_wind_down_unwinds() {
        assert_eq!(parse("gym session", &genres()).unwrap().arc, Arc::Build);
        assert_eq!(parse("time to relax", &genres()).unwrap().arc, Arc::Unwind);
    }

    #[test]
    fn a_genre_the_library_actually_has_is_matched() {
        let r = parse("put on some jazz for dinner", &genres()).unwrap();
        let g = r.filters.genres.expect("genre should be picked up");
        assert_eq!(g.include, vec!["Jazz".to_string()]);
    }

    /// Live regression, found on canary. "Dance" matched the party profile's
    /// keyword (bpm 110-125, energy 0.65-1.0) *and* a genre literally named
    /// "Dance" — and the two, ANDed together, matched zero tracks in a real
    /// library, for a perfectly reasonable request. The genre must survive;
    /// the profile's guessed bpm/energy must not, because it isn't ground
    /// truth about this library's "Dance"-tagged tracks the way the genre tag
    /// itself is.
    #[test]
    fn a_word_that_is_both_an_activity_and_a_real_genre_keeps_the_genre_not_the_guess() {
        let library_genres = vec!["Dance".to_string(), "Jazz".to_string()];
        let r = parse("dance disco for 2 hours", &library_genres).unwrap();

        let g = r.filters.genres.expect("the real genre must still match");
        assert_eq!(g.include, vec!["Dance".to_string()]);
        assert_eq!(
            r.filters.bpm, None,
            "the profile's guessed bpm must not stack with a ground-truth genre match"
        );
        assert_eq!(
            r.filters.energy, None,
            "the profile's guessed energy must not stack with a ground-truth genre match"
        );
        // The arc is sequencing-only — it can't empty a result — so it still
        // applies.
        assert_eq!(r.arc, Arc::Build);
    }

    /// The fix must not be so broad it breaks the case it has to keep working:
    /// two *different* words, one for the activity and one for the genre,
    /// still combine — that combination is the point of §4.2's filters, and
    /// nothing here shares a trigger word with anything in `genres()`.
    #[test]
    fn an_unrelated_genre_and_activity_still_combine() {
        let r = parse("put on some jazz for dinner", &genres()).unwrap();
        assert_eq!(r.filters.energy, Some([0.15, 0.5]), "dinner's profile must still apply");
        assert_eq!(r.arc, Arc::Unwind);
    }

    #[test]
    fn library_qualifiers_are_understood_without_any_audio_data() {
        let r = parse("play my forgotten favourites", &genres()).unwrap();
        assert_eq!(r.filters.not_played_days, Some(365));
        assert_eq!(r.filters.starred, Some(true));

        let r = parse("something new I have never played", &genres()).unwrap();
        assert_eq!(r.filters.added_days, Some(30));
    }

    /// The important half of the contract. A baseline that guesses is worse
    /// than one that declines: a confident playlist for a misread request is
    /// not something the user can debug.
    #[test]
    fn an_unrecognised_request_is_refused_rather_than_guessed() {
        for s in [
            "",
            "hello",
            "do the thing",
            "something for being done with something, but not sad about it",
        ] {
            assert_eq!(
                parse(s, &genres()),
                Err(IntentError::NotUnderstood),
                "{s:?} should not have produced a rule"
            );
        }
    }

    /// Whatever the sentence says, the output must be a rule the resolver can
    /// run. This is the boundary a model will later sit behind.
    #[test]
    fn every_rule_this_produces_is_valid() {
        for s in [
            "pilates for 2 hours",
            "running for 45 minutes",
            "party",
            "jazz for dinner",
            "relax for 600 minutes",
            "workout for 5000 minutes",
        ] {
            if let Ok(r) = parse(s, &genres()) {
                assert!(r.validate().is_ok(), "{s:?} produced an invalid rule");
            }
        }
    }

    /// An absurd duration is clamped rather than refused — the intent is clear
    /// even when the number is not.
    #[test]
    fn an_absurd_duration_is_clamped_to_something_runnable() {
        let r = parse("workout for 99999 minutes", &genres()).unwrap();
        match r.limit {
            Limit::Duration { minutes } => {
                assert!(minutes <= crate::music::rules::MAX_MINUTES)
            }
            other => panic!("expected a duration, got {other:?}"),
        }
    }
}
