// SPDX-License-Identifier: AGPL-3.0-or-later
//! Chapter marks inside an audio file (M4B, M4A, MP3 with chapter frames),
//! read with `ffprobe`. One reader for folder books and for music tracks
//! (docs/concerts-plan.md).

use std::path::Path;

/// The marks inside one file, as (start in seconds within the file, title).
/// Empty when the file has none or `ffprobe` is missing or fails: chapters
/// are extra, never a reason to refuse a file.
pub fn embedded(path: &Path) -> Vec<(f64, Option<String>)> {
    let Ok(out) = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_chapters", "-of", "json"])
        .arg(path)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse(&out.stdout)
}

fn parse(json: &[u8]) -> Vec<(f64, Option<String>)> {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(json) else { return Vec::new() };
    json.get("chapters")
        .and_then(|c| c.as_array())
        .map(|marks| {
            marks
                .iter()
                .filter_map(|c| {
                    let start = c.get("start_time")?.as_str()?.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0)?;
                    let title = c
                        .get("tags")
                        .and_then(|t| t.get("title"))
                        .and_then(|t| t.as_str())
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty());
                    Some((start, title))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A track's chapters as stored: (title, start in ms), in order. Fewer than two
/// marks are no chapters — a single mark only says where the file begins.
pub fn for_track(marks: Vec<(f64, Option<String>)>) -> Vec<(String, i64)> {
    if marks.len() < 2 {
        return Vec::new();
    }
    let mut marks = marks;
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    marks
        .into_iter()
        .enumerate()
        .map(|(i, (start, title))| (title.unwrap_or_else(|| format!("Chapter {}", i + 1)), (start * 1000.0).round() as i64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffprobe_output() {
        let json = br#"{"chapters":[
            {"id":0,"start_time":"0.000000","tags":{"title":"Intro"}},
            {"id":1,"start_time":"157.000000","tags":{"title":" The Emptiness Machine "}},
            {"id":2,"start_time":"377.500000","tags":{}}
        ]}"#;
        let marks = parse(json);
        assert_eq!(marks.len(), 3);
        assert_eq!(marks[1], (157.0, Some("The Emptiness Machine".into())));
        assert_eq!(marks[2].1, None);
        let stored = for_track(marks);
        assert_eq!(stored[1], ("The Emptiness Machine".into(), 157_000));
        assert_eq!(stored[2], ("Chapter 3".into(), 377_500));
    }

    #[test]
    fn one_mark_is_no_chapters() {
        assert!(for_track(vec![(0.0, Some("Whole file".into()))]).is_empty());
        assert!(parse(b"not json").is_empty());
    }

    /// A real file, made by ffmpeg the way an uploader writes its marks. Skipped
    /// where ffmpeg is not installed.
    #[test]
    fn reads_marks_from_an_m4a() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("chapters.txt");
        std::fs::write(
            &meta,
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=One\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=1000\nEND=2000\ntitle=Two\n",
        )
        .unwrap();
        let file = dir.path().join("concert.m4a");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-i"])
            .arg(&meta)
            .args(["-map_metadata", "1", "-c:a", "aac", "-t", "2"])
            .arg(&file)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("ffmpeg not available; skipped");
            return;
        }
        let stored = for_track(embedded(&file));
        assert_eq!(stored, vec![("One".to_string(), 0), ("Two".to_string(), 1000)]);
    }
}
