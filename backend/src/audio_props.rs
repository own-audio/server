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
    /// The container the headers say the file is, whatever its name: `mp3`, `mp4`, `adts`,
    /// `flac`, `ogg`, `wav`, `aiff`, `asf` (WMA), `ape`, `mpc`, `wavpack`… Read, not stored:
    /// it is what corrects `media_objects.content_type` when a file is not what its name says.
    #[serde(skip)]
    #[sqlx(default)]
    pub container: Option<String>,
}

impl AudioProps {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The content type the container calls for, where it is one the server knows.
    pub fn content_type(&self) -> Option<&'static str> {
        Some(match self.container.as_deref()? {
            "mp3" => "audio/mpeg",
            "mp4" => "audio/mp4",
            "adts" => "audio/aac",
            "flac" => "audio/flac",
            "ogg" => "audio/ogg",
            "wav" => "audio/wav",
            "aiff" => "audio/aiff",
            "asf" => "audio/x-ms-wma",
            "ape" => "audio/x-ape",
            "mpc" => "audio/x-musepack",
            "wavpack" => "audio/x-wavpack",
            _ => return None,
        })
    }

    /// The content type to record instead of `recorded`, when the file's own headers say it
    /// is something else: a WMA named `.mp3`, an MP3 named `.wav`, from an old rip. Spellings
    /// of the same thing (`audio/x-m4a` and `audio/mp4`) are left alone.
    pub fn corrected_content_type(&self, recorded: &str) -> Option<&'static str> {
        let real = self.content_type()?;
        (family(real) != family(recorded)).then_some(real)
    }
}

/// What a content type is a spelling of, so `audio/x-flac` and `audio/flac` are one thing.
fn family(content_type: &str) -> &'static str {
    match content_type.to_ascii_lowercase().as_str() {
        "audio/mpeg" | "audio/mp3" | "audio/mpeg3" | "audio/x-mpeg" => "mpeg",
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/m4b" | "audio/x-m4b" | "audio/mp4a-latm" => "mp4",
        "audio/aac" | "audio/aacp" | "audio/x-aac" => "aac",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/ogg" | "audio/opus" | "audio/vorbis" | "application/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => "wav",
        "audio/aiff" | "audio/x-aiff" => "aiff",
        "audio/x-ms-wma" | "audio/x-ms-asf" | "video/x-ms-asf" => "wma",
        "audio/x-ape" | "audio/ape" => "ape",
        "audio/x-musepack" | "audio/musepack" => "mpc",
        "audio/x-wavpack" | "audio/wavpack" => "wavpack",
        _ => "other",
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

    let container = match file_type {
        FileType::Mpeg => "mp3",
        FileType::Mp4 => "mp4",
        FileType::Aac => "adts",
        FileType::Aiff => "aiff",
        FileType::Ape => "ape",
        FileType::Flac => "flac",
        FileType::Mpc => "mpc",
        FileType::Opus | FileType::Vorbis | FileType::Speex => "ogg",
        FileType::Wav => "wav",
        FileType::WavPack => "wavpack",
        _ => return None,
    };
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
        // A WAV is a container: usually PCM, but old rips hold MPEG audio in one (format
        // tag 0x55), which a player expecting PCM cannot read.
        FileType::Wav => {
            use lofty::iff::wav::{WavFile, WavFormat};
            let mut f = std::fs::File::open(path).ok()?;
            WavFile::read_from(&mut f, options)
                .ok()
                .map(|w| match w.properties().format() {
                    WavFormat::PCM | WavFormat::IEEE_FLOAT => "wav",
                    WavFormat::Other(0x50) => "mp2",
                    WavFormat::Other(0x55) => "mp3",
                    WavFormat::Other(0x02) => "adpcm",
                    WavFormat::Other(0x06) => "alaw",
                    WavFormat::Other(0x07) => "mulaw",
                    WavFormat::Other(_) => "wav",
                })
                .unwrap_or("wav")
        }
        FileType::WavPack => "wavpack",
        _ => return None,
    };
    Some(AudioProps {
        codec: Some(codec.to_string()),
        sample_rate_hz: p.sample_rate().filter(|r| *r > 0).map(|r| r as i32),
        bit_depth: p.bit_depth().filter(|b| *b > 0).map(i16::from),
        channels: p.channels().filter(|c| *c > 0).map(i16::from),
        bitrate_kbps: p.audio_bitrate().or(p.overall_bitrate()).filter(|b| *b > 0).map(|b| b as i32),
        container: Some(container.to_string()),
    })
}

fn from_ffprobe(path: &Path) -> AudioProps {
    let Ok(out) = std::process::Command::new("ffprobe")
        .args([
            "-v", "error", "-select_streams", "a:0",
            "-show_entries", "stream=codec_name,sample_rate,channels,bits_per_raw_sample,bit_rate:format=bit_rate,format_name",
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
    // ffprobe names a demuxer, sometimes several ("mov,mp4,m4a,3gp,3g2,mj2"): the first says
    // what the file is.
    let container = v
        .get("format")
        .and_then(|f| f.get("format_name"))
        .and_then(|n| n.as_str())
        .and_then(|n| n.split(',').next())
        .map(|n| match n {
            "mov" => "mp4",
            "aac" => "adts",
            "aiff" => "aiff",
            other => other,
        })
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    AudioProps {
        codec,
        sample_rate_hz: num(stream.and_then(|s| s.get("sample_rate"))).map(|n| n as i32),
        bit_depth: num(stream.and_then(|s| s.get("bits_per_raw_sample"))).map(|n| n as i16),
        channels: num(stream.and_then(|s| s.get("channels"))).map(|n| n as i16),
        bitrate_kbps: bitrate.map(|b| ((b + 500) / 1000) as i32),
        container,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffprobe_strings_and_names_wma_plainly() {
        let json = br#"{"streams":[{"codec_name":"wmav2","sample_rate":"44100","channels":2,"bit_rate":"128000","bits_per_raw_sample":"N/A"}],"format":{"bit_rate":"129000","format_name":"asf"}}"#;
        assert_eq!(
            parse_ffprobe(json),
            AudioProps { codec: Some("wma".into()), sample_rate_hz: Some(44100), bit_depth: None, channels: Some(2), bitrate_kbps: Some(128), container: Some("asf".into()) }
        );
    }

    #[test]
    fn a_file_that_is_not_what_its_name_says_gets_the_content_type_of_what_it_is() {
        let wma = parse_ffprobe(br#"{"streams":[{"codec_name":"wmapro","sample_rate":"48000","channels":2}],"format":{"format_name":"asf"}}"#);
        assert_eq!(wma.content_type(), Some("audio/x-ms-wma"));
        assert_eq!(wma.corrected_content_type("audio/mpeg"), Some("audio/x-ms-wma"), "a WMA named .mp3");
        assert_eq!(wma.corrected_content_type("audio/x-ms-wma"), None);
        let m4a = parse_ffprobe(br#"{"streams":[{"codec_name":"aac"}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}}"#);
        assert_eq!(m4a.content_type(), Some("audio/mp4"));
        assert_eq!(m4a.corrected_content_type("audio/x-m4a"), None, "a spelling of the same thing");
        assert_eq!(m4a.corrected_content_type("audio/mpeg"), Some("audio/mp4"), "an M4A named .mp3");
        let odd = parse_ffprobe(br#"{"streams":[{"codec_name":"pcm_s16le"}],"format":{"format_name":"sox"}}"#);
        assert_eq!(odd.content_type(), None, "a container the server has no name for changes nothing");
        assert_eq!(AudioProps::default().corrected_content_type("audio/mpeg"), None);
    }

    #[test]
    fn falls_back_to_the_file_bitrate() {
        let json = br#"{"streams":[{"codec_name":"wmalossless","sample_rate":"96000","channels":2,"bits_per_raw_sample":"24"}],"format":{"bit_rate":"2116800"}}"#;
        let p = parse_ffprobe(json);
        assert_eq!((p.codec.as_deref(), p.bit_depth, p.bitrate_kbps), (Some("wma lossless"), Some(24), Some(2117)));
    }

    #[test]
    fn a_wav_holding_mp3_says_so() {
        let available = std::process::Command::new("ffmpeg").arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !available {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("own-audio-props-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("rip.wav");
        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-c:a", "libmp3lame", "-f", "wav"])
            .arg(&wav)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
        let p = read(&wav);
        assert_eq!((p.container.as_deref(), p.codec.as_deref()), (Some("wav"), Some("mp3")));
        assert_eq!(p.corrected_content_type("audio/wav"), None, "named right");
        // The same bytes named .mp3, as such rips often are.
        let named_mp3 = dir.join("rip.mp3");
        std::fs::copy(&wav, &named_mp3).unwrap();
        assert_eq!(read(&named_mp3).corrected_content_type("audio/mpeg"), Some("audio/wav"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_readable_is_empty() {
        assert!(parse_ffprobe(b"not json").is_empty());
        assert!(read(Path::new("/nonexistent/file.mp3")).is_empty());
    }
}
