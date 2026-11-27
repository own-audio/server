// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a file's own name and its folder's name say, for songs whose tags say
//! nothing. A song with no title tag gets its file name as its title at import
//! ("01 - Angel Of Death"), and searching MusicBrainz for that finds nothing;
//! the folder ("(1986) Slayer - Reign In Blood") often names the record outright.
//!
//! Only ever a hint: what a run finds from names alone is reviewed, never sure.

/// What the names gave. Every field is a guess.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FromNames {
    pub track_number: Option<i32>,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
}

/// Whether `title` is what the import made of the file's name: the song had no
/// title tag. `rel_path` is the file's path in its library folder.
pub fn title_is_file_name(title: &str, rel_path: &str) -> bool {
    let file = rel_path.rsplit('/').next().unwrap_or(rel_path);
    !title.trim().is_empty() && crate::music::infer_title(file) == title
}

/// A title that names nothing: "Track 01", "Audiotrack 3", "Unknown", "07".
/// Searching for one finds recordings literally called that.
pub fn is_generic_title(title: &str) -> bool {
    let t = title.trim().to_lowercase();
    let t = t.trim_matches(|c: char| !c.is_alphanumeric());
    if t.is_empty() || t.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    const WORDS: [&str; 12] = [
        "track", "audiotrack", "audio track", "piste", "pista", "titel", "stopa", "skladba", "untitled",
        "unknown", "no title", "bez názvu",
    ];
    WORDS.iter().any(|w| {
        t.strip_prefix(w).is_some_and(|rest| {
            rest.trim_matches(|c: char| c.is_whitespace() || c == '#' || c == '_' || c == '-' || c == '.')
                .chars()
                .all(|c| c.is_ascii_digit())
        })
    })
}

/// A file name without its extension, read as `[NN][ - ][Artist - ]Title`.
/// "01 - Angel Of Death", "09. Mary Jane", "03_GUANO - BONUS",
/// "Truth hurts-Addictive(feat.RAKIM)".
pub fn parse_file(stem: &str) -> FromNames {
    let s = stem.replace('_', " ");
    let s = s.trim();
    let (track_number, rest) = split_track_number(s);
    let rest = rest.trim();
    let (artist, title) = match split_dash(rest) {
        Some((a, t)) if !a.is_empty() && !t.is_empty() => (Some(a.to_string()), t.to_string()),
        _ => (None, rest.to_string()),
    };
    FromNames {
        track_number,
        artist,
        title: Some(collapse(&title)).filter(|t| !t.is_empty()),
        ..Default::default()
    }
}

/// An album folder's name: "(1986) Slayer - Reign In Blood", "[1963] The
/// Freewheelin' Bob Dylan", "Alanis Morissette - Jagged Little Pill",
/// "Slayer - Reign In Blood (1986) [FLAC]". Release notes in brackets
/// ("[FLAC]", "Mp3 320kbps", "[PMEDIA]") are dropped.
pub fn parse_folder(name: &str) -> FromNames {
    let mut year = None;
    let mut kept = String::with_capacity(name.len());
    // Bracketed parts: a year is kept as the year, anything else is a release note.
    let mut depth = 0usize;
    let mut inner = String::new();
    for c in name.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 {
                    inner.clear();
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(y) = as_year(inner.trim()) {
                        year = year.or(Some(y));
                    }
                    kept.push(' ');
                }
            }
            c if depth > 0 => inner.push(c),
            c => kept.push(c),
        }
    }
    let words: Vec<&str> = kept.split_whitespace().filter(|w| !is_release_note(w)).collect();
    let mut s = words.join(" ");
    // "1986 - Slayer - Reign In Blood": a leading year before a dash.
    if let Some((first, rest)) = split_dash(&s) {
        if let Some(y) = as_year(first) {
            year = year.or(Some(y));
            s = rest.to_string();
        }
    }
    let s = s.trim().trim_matches(|c: char| c == '-' || c == '.').trim().to_string();
    let (album_artist, album) = match split_dash(&s) {
        Some((a, b)) if !a.is_empty() && !b.is_empty() => (Some(a.to_string()), b.to_string()),
        _ => (None, s.clone()),
    };
    FromNames {
        album_artist,
        album: Some(collapse(&album.replace('`', "'"))).filter(|a| !a.is_empty()),
        year,
        ..Default::default()
    }
}

/// "NN", "NN.", "NN -", "NN)" at the start, not followed by nothing; 1–3 digits
/// only, so "1979" and "2 X 4" style names are left alone when no separator follows.
fn split_track_number(s: &str) -> (Option<i32>, &str) {
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return (None, s);
    }
    let rest = &s[digits..];
    let after = rest.trim_start_matches([' ', '.', '-', ')', ']']);
    // "7 Seconds" stays a title: a bare space is a separator only when something
    // clearer follows too, or the number has a leading zero ("07 Seconds").
    let separated = rest.starts_with(['.', '-', ')', ']'])
        || rest.trim_start().starts_with(['-', '.'])
        || (rest.starts_with(' ') && s.starts_with('0'));
    if !separated || after.trim().is_empty() {
        return (None, s);
    }
    (s[..digits].parse().ok(), after)
}

/// Split at the first " - " (or a bare "-" between words when there is no
/// spaced one), the separator file and folder names use between artist and title.
fn split_dash(s: &str) -> Option<(&str, &str)> {
    if let Some(i) = s.find(" - ") {
        return Some((s[..i].trim(), s[i + 3..].trim()));
    }
    let i = s.find('-')?;
    let (a, b) = (&s[..i], &s[i + 1..]);
    // "Truth hurts-Addictive": letters on both sides; not "Jay-Z" (one word each side).
    if a.contains(' ') && !b.trim().is_empty() && a.chars().last().is_some_and(char::is_alphanumeric) {
        Some((a.trim(), b.trim()))
    } else {
        None
    }
}

fn as_year(s: &str) -> Option<i32> {
    (s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
        .filter(|y| (1900..=2100).contains(y))
}

fn is_release_note(word: &str) -> bool {
    let w = word.to_lowercase();
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    matches!(w, "mp3" | "flac" | "aac" | "m4a" | "ogg" | "wav" | "alac" | "kbps" | "cbr" | "vbr" | "web" | "cd" | "rip")
        || (w.ends_with("kbps") && w[..w.len() - 4].chars().all(|c| c.is_ascii_digit()))
        || (w.ends_with('k') && w.len() > 1 && w[..w.len() - 1].chars().all(|c| c.is_ascii_digit()))
        || (word != "-" && word.chars().all(|c| !c.is_alphanumeric()))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_give_number_and_title() {
        let f = parse_file("01 - Angel Of Death");
        assert_eq!((f.track_number, f.artist.as_deref(), f.title.as_deref()), (Some(1), None, Some("Angel Of Death")));
        let f = parse_file("09. Mary Jane");
        assert_eq!((f.track_number, f.title.as_deref()), (Some(9), Some("Mary Jane")));
        let f = parse_file("12 - Honey, Just Allow Me One More Chance");
        assert_eq!((f.track_number, f.title.as_deref()), (Some(12), Some("Honey, Just Allow Me One More Chance")));
        let f = parse_file("03_GUANO - BONUS");
        assert_eq!((f.track_number, f.artist.as_deref(), f.title.as_deref()), (Some(3), Some("GUANO"), Some("BONUS")));
        let f = parse_file("03 I Want to Know What Love Is");
        assert_eq!((f.track_number, f.title.as_deref()), (Some(3), Some("I Want to Know What Love Is")));
    }

    #[test]
    fn file_names_give_artist_and_title() {
        let f = parse_file("Truth hurts-Addictive(feat.RAKIM)");
        assert_eq!((f.artist.as_deref(), f.title.as_deref()), (Some("Truth hurts"), Some("Addictive(feat.RAKIM)")));
        let f = parse_file("Slayer - Angel Of Death");
        assert_eq!((f.track_number, f.artist.as_deref(), f.title.as_deref()), (None, Some("Slayer"), Some("Angel Of Death")));
    }

    #[test]
    fn titles_that_start_with_a_number_stay() {
        assert_eq!(parse_file("7 Seconds").title.as_deref(), Some("7 Seconds"));
        assert_eq!(parse_file("7 Seconds").track_number, None);
        assert_eq!(parse_file("1979").title.as_deref(), Some("1979"));
        assert_eq!(parse_file("99 Luftballons").track_number, None);
        assert_eq!(parse_file("Jay-Z").title.as_deref(), Some("Jay-Z"));
    }

    #[test]
    fn folder_names_give_year_artist_album() {
        let f = parse_folder("(1986) Slayer - Reign In Blood");
        assert_eq!((f.year, f.album_artist.as_deref(), f.album.as_deref()), (Some(1986), Some("Slayer"), Some("Reign In Blood")));
        let f = parse_folder("[1963] The Freewheelin` Bob Dylan");
        assert_eq!((f.year, f.album_artist.as_deref(), f.album.as_deref()), (Some(1963), None, Some("The Freewheelin' Bob Dylan")));
        let f = parse_folder("Alanis Morissette - Jagged Little Pill");
        assert_eq!((f.album_artist.as_deref(), f.album.as_deref()), (Some("Alanis Morissette"), Some("Jagged Little Pill")));
        let f = parse_folder("GUANO APES - PROUD LIKE A GOD");
        assert_eq!((f.album_artist.as_deref(), f.album.as_deref()), (Some("GUANO APES"), Some("PROUD LIKE A GOD")));
        let f = parse_folder("Slayer - Reign In Blood (1986) [FLAC]");
        assert_eq!((f.year, f.album_artist.as_deref(), f.album.as_deref()), (Some(1986), Some("Slayer"), Some("Reign In Blood")));
        let f = parse_folder("1996 - Slayer - Undisputed Attitude");
        assert_eq!((f.year, f.album_artist.as_deref(), f.album.as_deref()), (Some(1996), Some("Slayer"), Some("Undisputed Attitude")));
    }

    #[test]
    fn release_notes_in_folder_names_are_dropped() {
        let f = parse_folder("'80s Soft Rock Essentials Mp3~320   kbps~ Beats⭐");
        assert_eq!(f.album.as_deref().map(|a| a.starts_with("'80s Soft Rock Essentials")), Some(true), "{f:?}");
        let f = parse_folder("Fifth Element - Soundtrack");
        assert_eq!((f.album_artist.as_deref(), f.album.as_deref()), (Some("Fifth Element"), Some("Soundtrack")));
    }

    #[test]
    fn generic_titles() {
        for t in ["Track 01", "Track #07", "track_3", "Audiotrack 12", "Unknown", "Untitled", "07", "Stopa 4", "Skladba 2"] {
            assert!(is_generic_title(t), "{t}");
        }
        for t in ["Trackmaster", "Unknown Pleasures", "Track of the Cat", "Angel Of Death", "1979 Revisited"] {
            assert!(!is_generic_title(t), "{t}");
        }
    }

    #[test]
    fn a_title_made_from_the_file_name_is_recognised() {
        assert!(title_is_file_name("01   Angel Of Death", "x/(1986) Slayer - Reign In Blood/01 - Angel Of Death.mp3"));
        assert!(!title_is_file_name("Angel Of Death", "x/(1986) Slayer - Reign In Blood/01 - Angel Of Death.mp3"));
    }
}
