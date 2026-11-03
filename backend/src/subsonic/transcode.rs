// SPDX-License-Identifier: AGPL-3.0-or-later
//! A smaller stream on request: Subsonic `stream` with `maxBitRate` and/or
//! `format`. ffmpeg reads the original where it is — a file on this disk, or
//! S3 by a link — and its output goes straight to the client: nothing is
//! written to disk and memory stays at a chunk or two whatever the file size.
//!
//! Mobile players ask for this to save data; it is also how a client that
//! carries no decoders of its own plays a format its platform can't (the
//! server makes MP3 or AAC of it).

use crate::storage::ObjectStore;
use axum::body::{Body, Bytes};
use axum::http::{StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// At most this many ffmpeg processes at once; a request beyond it gets the
/// original file instead of waiting. A Raspberry Pi manages a few MP3 encodes
/// faster than real time; a busy server shouldn't queue listeners behind them.
static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| {
    let n = std::env::var("SUBSONIC__MAX_TRANSCODES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4);
    Semaphore::new(n)
});

/// How long a link into S3 stays good for ffmpeg: the longest transcode, with room.
const LINK_SECS: u64 = 6 * 3600;
/// How long ffmpeg has to produce its first bytes before the original is sent
/// instead. Longer than any player waits: a transcode this slow to start is
/// lost either way, and the slot is better freed.
const FIRST_BYTES: Duration = Duration::from_secs(20);

/// Where ffmpeg reads the original. Both can be seeked — a file by offset, a
/// link by `Range` — which the pipe from storage used before could not: an
/// M4A whose index (`moov`) follows its audio was unreadable through it (the
/// transcode was an empty 200), and a start part-way in meant decoding
/// everything before it first, which for an hour into an audiobook was longer
/// than a player waits for the first byte.
pub enum Input {
    File(PathBuf),
    /// Signed for the store as the server reaches it, not as players do
    /// (`ObjectStore::internal_get_url`).
    Url(String),
}

impl Input {
    fn arg(&self) -> String {
        match self {
            Input::File(path) => path.to_string_lossy().into_owned(),
            Input::Url(url) => url.clone(),
        }
    }
}

/// The original of `key`, as ffmpeg can read it.
pub async fn input(storage: &ObjectStore, key: &str) -> anyhow::Result<Input> {
    match storage.local_file(key) {
        Some(path) => Ok(Input::File(path?)),
        None => Ok(Input::Url(storage.internal_get_url(key, LINK_SECS).await?)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp3,
    /// ADTS, so it can be decoded as it arrives (an MP4 would need its index first).
    Aac,
}

impl Format {
    fn content_type(self) -> &'static str {
        match self {
            Format::Mp3 => "audio/mpeg",
            Format::Aac => "audio/aac",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub format: Format,
    pub kbps: u32,
    pub offset_secs: u32,
}

/// Whether to transcode, and to what. `None` means send the original:
/// nothing was asked for, `format=raw`, an unknown format, or a bitrate limit
/// the original already meets (`source_kbps`, when known, from size and
/// duration).
pub fn plan(max_bit_rate: Option<u32>, format: Option<&str>, time_offset: Option<u32>, source_kbps: Option<u32>) -> Option<Plan> {
    let limit = max_bit_rate.filter(|&k| k > 0);
    let format = match format.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") => None,
        Some("raw") => return None,
        Some("mp3") => Some(Format::Mp3),
        Some("aac") | Some("m4a") => Some(Format::Aac),
        // Opus, Vorbis, FLAC… aren't offered: the point is a stream any player
        // decodes, and asking for one of those gets the original instead.
        Some(_) => return None,
    };
    match (format, limit) {
        (None, None) => None,
        // Only a limit: worth it only when the original is over it.
        (None, Some(limit)) => {
            if source_kbps.is_some_and(|s| s <= limit) {
                None
            } else {
                Some(Plan { format: Format::Mp3, kbps: clamp(limit), offset_secs: time_offset.unwrap_or(0) })
            }
        }
        (Some(format), limit) => {
            Some(Plan { format, kbps: clamp(limit.unwrap_or(192)), offset_secs: time_offset.unwrap_or(0) })
        }
    }
}

fn clamp(kbps: u32) -> u32 {
    kbps.clamp(32, 320)
}

/// The sample rates a transcode may have: the source's where it is one of these, else the nearest.
const SAMPLE_RATES: &str = "8000|11025|12000|16000|22050|24000|32000|44100|48000";

/// The ffmpeg arguments for `plan`, reading `input` and writing stdout.
pub fn args(input: &str, plan: Plan) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"].map(String::from).to_vec();
    // Before the input: a file or a link is seeked to here (and trimmed to
    // the exact second from the nearest frame before it), not decoded up to it.
    if plan.offset_secs > 0 {
        args.extend(["-ss".into(), plan.offset_secs.to_string()]);
    }
    args.extend(["-i".into(), input.to_string()]);
    args.extend(["-map", "0:a:0", "-vn", "-map_metadata", "-1"].map(String::from));
    // At most 48 kHz and stereo: a 24-bit/96 kHz rip or a 5.1 mix gains nothing
    // from more in a stream made to be small, and not every player's AAC
    // decoder takes 96 kHz. A lower rate is kept as it is.
    args.extend(["-af".into(), format!("aformat=sample_rates={SAMPLE_RATES}:channel_layouts=mono|stereo")]);
    match plan.format {
        Format::Mp3 => args.extend(["-c:a", "libmp3lame", "-f", "mp3"].map(String::from)),
        Format::Aac => args.extend(["-c:a", "aac", "-f", "adts"].map(String::from)),
    }
    args.extend(["-b:a".into(), format!("{}k", plan.kbps), "pipe:1".into()]);
    args
}

/// Starts the transcode of `input`. `None` when every slot is taken, ffmpeg
/// can't be started, or it produces nothing (a file it can't read); the caller
/// then sends the original.
pub async fn start(input: Input, plan: Plan) -> Option<Response> {
    let permit = SLOTS.try_acquire().ok()?;
    let mut child = Command::new("ffmpeg")
        .args(args(&input.arg(), plan))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A listener who skips ahead drops the response; the process goes with it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| tracing::warn!(error = %err, "ffmpeg could not be started for a transcode"))
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    // Its complaints, read as they come so a chatty ffmpeg never stalls on a
    // full pipe; the first of them are shown when the transcode made nothing.
    let complaints = tokio::spawn(async move {
        let mut kept = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            if kept.len() < 4096 {
                kept.extend_from_slice(&buf[..n]);
            }
        }
        String::from_utf8_lossy(&kept).trim().to_string()
    });

    // ffmpeg's first bytes come before the answer: an input it can't read ends
    // here with none, and the original goes out instead of an empty 200.
    let mut first = vec![0u8; 64 * 1024];
    let n = match tokio::time::timeout(FIRST_BYTES, stdout.read(&mut first)).await {
        Ok(Ok(n)) => n,
        Ok(Err(_)) | Err(_) => 0,
    };
    if n == 0 {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let why = complaints.await.unwrap_or_default();
        tracing::warn!(reason = %why, "a transcode produced nothing; the original is sent");
        drop(permit);
        return None;
    }
    first.truncate(n);

    // The slot is held for as long as the process lives.
    tokio::spawn(async move {
        let _permit = permit;
        let _ = child.wait().await;
        let _ = complaints.await;
    });

    let rest = tokio_util::io::ReaderStream::new(stdout);
    let body = Body::from_stream(futures_util::stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(first)) }).chain(rest));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, plan.format.content_type())
        // The length isn't known until the encode ends, and a byte range of a
        // stream that doesn't exist yet can't be served.
        .header(header::ACCEPT_RANGES, "none")
        .body(body)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_asked_is_the_original() {
        assert_eq!(plan(None, None, None, Some(900)), None);
        assert_eq!(plan(Some(0), None, None, Some(900)), None);
        assert_eq!(plan(Some(128), Some("raw"), None, Some(900)), None);
    }

    #[test]
    fn a_limit_the_original_meets_is_the_original() {
        assert_eq!(plan(Some(192), None, None, Some(128)), None);
        assert_eq!(
            plan(Some(128), None, None, Some(900)),
            Some(Plan { format: Format::Mp3, kbps: 128, offset_secs: 0 })
        );
        // Unknown source bitrate: transcode to be safe.
        assert_eq!(
            plan(Some(128), None, Some(30), None),
            Some(Plan { format: Format::Mp3, kbps: 128, offset_secs: 30 })
        );
    }

    #[test]
    fn a_format_always_transcodes_and_odd_ones_get_the_original() {
        assert_eq!(plan(None, Some("AAC"), None, Some(96)), Some(Plan { format: Format::Aac, kbps: 192, offset_secs: 0 }));
        assert_eq!(plan(Some(1000), Some("mp3"), None, None), Some(Plan { format: Format::Mp3, kbps: 320, offset_secs: 0 }));
        assert_eq!(plan(Some(64), Some("opus"), None, None), None);
    }

    #[test]
    fn ffmpeg_arguments() {
        let a = args("/music/a.flac", Plan { format: Format::Mp3, kbps: 128, offset_secs: 0 });
        assert!(a.windows(2).any(|w| w == ["-c:a", "libmp3lame"]));
        assert!(a.windows(2).any(|w| w == ["-b:a", "128k"]));
        assert!(a.windows(2).any(|w| w == ["-i", "/music/a.flac"]));
        assert!(!a.contains(&"-ss".to_string()));
        let b = args("https://store.example/o?x=1", Plan { format: Format::Aac, kbps: 96, offset_secs: 45 });
        assert!(b.windows(2).any(|w| w == ["-ss", "45"]));
        assert!(b.windows(2).any(|w| w == ["-f", "adts"]));
        assert!(b.iter().any(|a| a.starts_with("aformat=sample_rates=") && a.ends_with("|48000:channel_layouts=mono|stereo")));
        let (ss, i) = (b.iter().position(|a| a == "-ss").unwrap(), b.iter().position(|a| a == "-i").unwrap());
        assert!(ss < i, "the seek goes before the input, so a seekable input is seeked rather than decoded up to the offset");
    }

    /// The real ffmpeg, where the machine has one (CI and the image do).
    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    /// A 20 s tone as an M4A written the default way: its `moov` index after the audio.
    fn m4a_with_its_index_last() -> PathBuf {
        let path = std::env::temp_dir().join(format!("own-audio-transcode-{}.m4a", uuid::Uuid::new_v4()));
        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=20", "-c:a", "aac", "-b:a", "96k"])
            .arg(&path)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
        let head = std::fs::read(&path).unwrap();
        let moov = head.windows(4).position(|w| w == b"moov").unwrap();
        let mdat = head.windows(4).position(|w| w == b"mdat").unwrap();
        assert!(moov > mdat, "the fixture's index must follow its audio for this test to mean anything");
        path
    }

    async fn body_of(response: Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), 64 << 20).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn an_m4a_with_its_index_last_transcodes_from_a_file() {
        if !ffmpeg_available() {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let path = m4a_with_its_index_last();
        let response = start(Input::File(path.clone()), Plan { format: Format::Aac, kbps: 64, offset_secs: 0 }).await.expect("a transcode starts");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/aac");
        let whole = body_of(response).await;
        assert!(whole.len() > 100_000, "20 s at 64 kbps: {} bytes", whole.len());
        assert_eq!(whole[0], 0xFF, "ADTS starts with a sync word");
        assert_eq!(whole[1] & 0xF6, 0xF0);

        // A start part-way in: seeked, and what is left is shorter.
        let response = start(Input::File(path.clone()), Plan { format: Format::Aac, kbps: 64, offset_secs: 15 }).await.expect("a transcode starts");
        let tail = body_of(response).await;
        assert!(!tail.is_empty() && tail.len() < whole.len() / 2, "5 s of 20: {} of {} bytes", tail.len(), whole.len());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_hi_res_source_is_sent_at_48_khz() {
        if !ffmpeg_available() {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let path = std::env::temp_dir().join(format!("own-audio-transcode-{}.flac", uuid::Uuid::new_v4()));
        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=96000:duration=3", "-c:a", "flac"])
            .arg(&path)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
        let response = start(Input::File(path.clone()), Plan { format: Format::Aac, kbps: 128, offset_secs: 0 }).await.expect("a transcode starts");
        let adts = body_of(response).await;
        // ADTS header byte 2, bits 2–5: the sampling frequency index; 3 is 48 000 Hz, 0 would be 96 000.
        assert_eq!((adts[2] >> 2) & 0x0F, 3, "sampling frequency index");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn an_input_ffmpeg_cannot_read_sends_the_original() {
        if !ffmpeg_available() {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let path = std::env::temp_dir().join(format!("own-audio-transcode-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, "not audio at all").unwrap();
        assert!(start(Input::File(path.clone()), Plan { format: Format::Mp3, kbps: 128, offset_secs: 0 }).await.is_none());
        let _ = std::fs::remove_file(path);
    }
}
