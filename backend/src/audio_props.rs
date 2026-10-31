// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a file's audio is — codec, sample rate, bit depth, channels, bitrate —
//! read from its headers when the file arrives, so a player can say what it
//! is playing. A file's name says little: `.mp3` files from old rips can be
//! MPEG Layer II, which some browsers cannot play.

use std::path::Path;

use serde::Serialize;
use utoipa::ToSchema;

/// Every field is optional: a format may not carry it (no bit depth in a lossy
/// file), or the reader may not know the format at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema, sqlx::FromRow)]
pub struct AudioProps {
    /// Lowercase codec name: `mp3`, `mp2`, `aac`, `alac`, `flac`, `opus`, `vorbis`,
    /// `wav`, `aiff`, `wma`, `wma lossless`… Unknown values may appear.
    pub codec: Option<String>,
    pub sample_rate_hz: Option<i32>,
    /// Bits per sample, for formats that have them (lossless, PCM).
    pub bit_depth: Option<i16>,
    pub channels: Option<i16>,
    /// The audio's bitrate, or the file's average where the format gives only that.
    pub bitrate_kbps: Option<i32>,
}

impl AudioProps {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Reads the properties, with `lofty` and, for what it cannot open (WMA), ffprobe.
/// Never fails: a file nothing can read gives empty properties.
pub fn read(path: &Path) -> AudioProps {
    from_lofty(path).unwrap_or_else(|| from_ffprobe(path))
}

fn from_lofty(path: &Path) -> Option<AudioProps> {
    use lofty::config::ParseOptions;
    use lofty::file::{AudioFile, FileType};

    let options = ParseOptions::new().read_tags(false).read_cover_art(false);
    // By content, not by name: a `.mp3` holding MPEG Layer II is still found.
    let probe = lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.options(options);
    let file_type = probe.file_type()?;
    let file = probe.read().ok()?;
    let p = file.properties();

    let codec = match file_type {
        FileType::Mpeg => {
            use lofty::mpeg::{Layer, MpegFile};
            let mut f = std::fs::File::open(path).ok()?;
            match MpegFile::read_from(&mut f, options).ok().map(|m| *m.properties().layer()) {
                Some(Layer::Layer1) => "mp1",
                Some(Layer::Layer2) => "mp2",
                _ => "mp3",
            }
        }
        FileType::Mp4 => {
            use lofty::mp4::{Mp4Codec, Mp4File};
            let mut f = std::fs::File::open(path).ok()?;
            match Mp4File::read_from(&mut f, options).ok().and_then(|m| m.properties().codec()) {
                Some(Mp4Codec::ALAC) => "alac",
                Some(Mp4Codec::MP3) => "mp3",
                Some(Mp4Codec::FLAC) => "flac",
                _ => "aac",
            }
        }
        FileType::Aac => "aac",
        FileType::Aiff => "aiff",
        FileType::Ape => "ape",
        FileType::Flac => "flac",
        FileType::Mpc => "mpc",
        FileType::Opus => "opus",
        FileType::Vorbis => "vorbis",
        FileType::Speex => "speex",
        FileType::Wav => "wav",
        FileType::WavPack => "wavpack",
        _ => return None,
    };
    Some(AudioProps {
        codec: Some(codec.to_string()),
        sample_rate_hz: p.sample_rate().filter(|r| *r > 0).map(|r| r as i32),
        bit_depth: p.bit_depth().filter(|b| *b > 0).map(i16::from),
        channels: p.channels().filter(|c| *c > 0).map(i16::from),
        bitrate_kbps: p.audio_bitrate().or(p.overall_bitrate()).filter(|b| *b > 0).map(|b| b as i32),
    })
}

fn from_ffprobe(path: &Path) -> AudioProps {
    let Ok(out) = std::process::Command::new("ffprobe")
        .args([
            "-v", "error", "-select_streams", "a:0",
            "-show_entries", "stream=codec_name,sample_rate,channels,bits_per_raw_sample,bit_rate:format=bit_rate",
            "-of", "json",
        ])
        .arg(path)
        .output()
    else {
        return AudioProps::default();
    };
    if !out.status.success() {
        return AudioProps::default();
    }
    parse_ffprobe(&out.stdout)
}

fn parse_ffprobe(json: &[u8]) -> AudioProps {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else { return AudioProps::default() };
    let stream = v.get("streams").and_then(|s| s.get(0));
    // ffprobe gives numbers as strings, and "N/A" where it has none.
    let num = |value: Option<&serde_json::Value>| -> Option<i64> {
        value.and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).filter(|n| *n > 0)
    };
    let codec = stream.and_then(|s| s.get("codec_name")).and_then(|c| c.as_str()).map(|c| {
        match c {
            "wmav1" | "wmav2" => "wma",
            "wmapro" => "wma pro",
            "wmalossless" => "wma lossless",
            other => other,
        }
        .to_string()
    });
    let bitrate = num(stream.and_then(|s| s.get("bit_rate"))).or_else(|| num(v.get("format").and_then(|f| f.get("bit_rate"))));
    AudioProps {
        codec,
        sample_rate_hz: num(stream.and_then(|s| s.get("sample_rate"))).map(|n| n as i32),
        bit_depth: num(stream.and_then(|s| s.get("bits_per_raw_sample"))).map(|n| n as i16),
        channels: num(stream.and_then(|s| s.get("channels"))).map(|n| n as i16),
        bitrate_kbps: bitrate.map(|b| ((b + 500) / 1000) as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffprobe_strings_and_names_wma_plainly() {
        let json = br#"{"streams":[{"codec_name":"wmav2","sample_rate":"44100","channels":2,"bit_rate":"128000","bits_per_raw_sample":"N/A"}],"format":{"bit_rate":"129000"}}"#;
        assert_eq!(
            parse_ffprobe(json),
            AudioProps { codec: Some("wma".into()), sample_rate_hz: Some(44100), bit_depth: None, channels: Some(2), bitrate_kbps: Some(128) }
        );
    }

    #[test]
    fn falls_back_to_the_file_bitrate() {
        let json = br#"{"streams":[{"codec_name":"wmalossless","sample_rate":"96000","channels":2,"bits_per_raw_sample":"24"}],"format":{"bit_rate":"2116800"}}"#;
        let p = parse_ffprobe(json);
        assert_eq!((p.codec.as_deref(), p.bit_depth, p.bitrate_kbps), (Some("wma lossless"), Some(24), Some(2117)));
    }

    #[test]
    fn nothing_readable_is_empty() {
        assert!(parse_ffprobe(b"not json").is_empty());
        assert!(read(Path::new("/nonexistent/file.mp3")).is_empty());
    }
}
