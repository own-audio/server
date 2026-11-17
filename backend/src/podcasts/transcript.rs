// SPDX-License-Identifier: AGPL-3.0-or-later
//! An episode's transcript for reading: the publisher's `<podcast:transcript>`, or the file an
//! owner uploaded with the episode, turned into timed lines (WebVTT, SRT) or plain text.

use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;
use utoipa::ToSchema;

/// Transcripts are text; anything this large is not one.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct Transcript {
    /// `cues` (timed lines from WebVTT or SRT) or `text` (plain text, or a format without timing).
    pub format: String,
    pub cues: Vec<Cue>,
    pub text: Option<String>,
}

static TIMING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:(\d+):)?(\d{1,2}):(\d{2})[.,](\d{3})\s*-->\s*(?:(\d+):)?(\d{1,2}):(\d{2})[.,](\d{3})").unwrap()
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
/// WebVTT's voice span, `<v Anna>` or `<v.loud Anna>`: who speaks the line.
static VOICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<v(?:\.[^\s>]*)?\s+([^>]+)>").unwrap());

/// A timed line with its speaker when the file names one (WebVTT voices, Podcast Index JSON).
#[derive(Debug, PartialEq)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
    pub text: String,
}

fn ms(h: Option<&str>, m: &str, s: &str, frac: &str) -> i64 {
    let n = |v: &str| v.parse::<i64>().unwrap_or(0);
    ((h.map(n).unwrap_or(0) * 60 + n(m)) * 60 + n(s)) * 1000 + n(frac)
}

fn clean(line: &str) -> String {
    let text = TAG.replace_all(line, "");
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// WebVTT or SRT into timed lines. YouTube's automatic captions repeat the previous line at
/// the top of each cue (rolling captions); a line equal to the last one kept is dropped, so
/// the transcript reads once through.
pub fn parse_cues(raw: &str) -> Vec<Cue> {
    segments_of_cues(raw).into_iter().map(|s| Cue { start_ms: s.start_ms, end_ms: s.end_ms, text: s.text }).collect()
}

fn segments_of_cues(raw: &str) -> Vec<Segment> {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues: Vec<Segment> = Vec::new();
    let mut last_line = String::new();
    for block in normalized.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|l| !l.is_empty());
        let Some(timing) = lines.by_ref().find(|l| l.contains("-->")) else { continue };
        let Some(t) = TIMING.captures(timing) else { continue };
        let start_ms = ms(t.get(1).map(|m| m.as_str()), &t[2], &t[3], &t[4]);
        let end_ms = ms(t.get(5).map(|m| m.as_str()), &t[6], &t[7], &t[8]);
        let mut kept = Vec::new();
        let mut speaker = None;
        for line in lines {
            if speaker.is_none() {
                speaker = VOICE.captures(line).map(|v| clean(&v[1])).filter(|v| !v.is_empty());
            }
            let text = clean(line);
            if text.is_empty() || text == last_line {
                continue;
            }
            last_line = text.clone();
            kept.push(text);
        }
        if !kept.is_empty() {
            cues.push(Segment { start_ms, end_ms, speaker, text: kept.join(" ") });
        }
    }
    cues
}

/// Podcast Index JSON (`{"segments": [{"startTime", "endTime", "body"}]}`) into timed lines.
fn parse_json(raw: &str) -> Option<Vec<Segment>> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let segments = value.get("segments")?.as_array()?;
    let secs = |v: Option<&serde_json::Value>| (v.and_then(|v| v.as_f64()).unwrap_or(0.0) * 1000.0) as i64;
    Some(
        segments
            .iter()
            .filter_map(|s| {
                let text = clean(s.get("body")?.as_str()?);
                let speaker = s.get("speaker").and_then(|v| v.as_str()).map(clean).filter(|v| !v.is_empty());
                (!text.is_empty()).then(|| Segment { start_ms: secs(s.get("startTime")), end_ms: secs(s.get("endTime")), speaker, text })
            })
            .collect(),
    )
}

/// Timed lines with their speakers, or (when the file has no timing) its plain text.
pub fn segments(raw: &str, mime: &str) -> (Vec<Segment>, Option<String>) {
    let kind = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let timed = match kind.as_str() {
        "text/vtt" | "application/srt" | "text/srt" | "application/x-subrip" => segments_of_cues(raw),
        "application/json" | "application/json+srv3" => parse_json(raw).unwrap_or_default(),
        // A file whose type nobody stated: timed if it looks it.
        _ if raw.contains("-->") => segments_of_cues(raw),
        _ => Vec::new(),
    };
    if timed.is_empty() {
        let text = raw.trim();
        return (timed, (!text.is_empty()).then(|| text.to_string()));
    }
    (timed, None)
}

pub fn parse(raw: &str, mime: &str) -> Transcript {
    let (timed, text) = segments(raw, mime);
    if timed.is_empty() {
        return Transcript { format: "text".into(), cues: Vec::new(), text };
    }
    let cues = timed.into_iter().map(|s| Cue { start_ms: s.start_ms, end_ms: s.end_ms, text: s.text }).collect();
    Transcript { format: "cues".into(), cues, text: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvtt_becomes_timed_lines_with_tags_and_entities_removed() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n00:00:00.320 --> 00:00:05.360\nAah, an abundance of&nbsp;\n resources, <i>untouched</i>.\n\n01:02:03.004 --> 01:02:04.500 align:start\n<v Anna>Hello &amp; welcome\n";
        let t = parse(vtt, "text/vtt");
        assert_eq!(t.format, "cues");
        assert_eq!(t.cues, vec![
            Cue { start_ms: 320, end_ms: 5360, text: "Aah, an abundance of resources, untouched.".into() },
            Cue { start_ms: 3_723_004, end_ms: 3_724_500, text: "Hello & welcome".into() },
        ]);
    }

    #[test]
    fn rolling_automatic_captions_read_once() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nso today we\n\n00:00:02.000 --> 00:00:03.000\nso today we\nlook at rivers\n\n00:00:03.000 --> 00:00:04.000\nlook at rivers\n";
        let t = parse(vtt, "text/vtt");
        assert_eq!(t.cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), vec!["so today we", "look at rivers"]);
    }

    #[test]
    fn srt_json_and_plain_text() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nFirst line\n\n2\n00:00:03,000 --> 00:00:04,000\nSecond\n";
        assert_eq!(parse(srt, "application/srt").cues.len(), 2);
        let json = r#"{"version":"1.0","segments":[{"startTime":1.5,"endTime":2.0,"body":"Hi"}]}"#;
        assert_eq!(parse(json, "application/json").cues, vec![Cue { start_ms: 1500, end_ms: 2000, text: "Hi".into() }]);
        let plain = parse("Just words.\n\nMore words.", "text/plain");
        assert_eq!((plain.format.as_str(), plain.text.as_deref()), ("text", Some("Just words.\n\nMore words.")));
        assert!(parse("", "text/plain").text.is_none());
    }

    #[test]
    fn speakers_come_from_webvtt_voices_and_json() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v.loud Anna Nováková>Hello</v>\n\n00:00:02.000 --> 00:00:03.000\nNo voice here\n";
        let (timed, text) = segments(vtt, "text/vtt");
        assert!(text.is_none());
        assert_eq!(timed[0], Segment { start_ms: 1000, end_ms: 2000, speaker: Some("Anna Nováková".into()), text: "Hello".into() });
        assert_eq!(timed[1].speaker, None);
        let json = r#"{"segments":[{"startTime":0.5,"endTime":1,"speaker":"Host","body":"Hi"}]}"#;
        assert_eq!(segments(json, "application/json").0[0].speaker.as_deref(), Some("Host"));
        assert_eq!(parse(vtt, "text/vtt").cues[0].text, "Hello", "v1's lines are unchanged");
    }
}
