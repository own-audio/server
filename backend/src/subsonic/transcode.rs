// SPDX-License-Identifier: AGPL-3.0-or-later
//! A smaller stream on request: Subsonic `stream` with `maxBitRate` and/or
//! `format`. The original is read from storage in chunks and piped through
//! ffmpeg, whose output goes straight to the client — nothing is written to
//! disk and memory stays at a chunk or two whatever the file size.
//!
//! Mobile players ask for this to save data; it is also how a client that
//! carries no decoders of its own plays a format its platform can't (the
//! server makes MP3 or AAC of it).

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use std::process::Stdio;
use std::sync::LazyLock;
use tokio::io::AsyncWriteExt;
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

/// The ffmpeg arguments for `plan`, reading stdin and writing stdout.
pub fn args(plan: Plan) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-i", "pipe:0"].map(String::from).to_vec();
    // After the input: stdin can't seek, so ffmpeg decodes and drops up to here.
    if plan.offset_secs > 0 {
        args.extend(["-ss".into(), plan.offset_secs.to_string()]);
    }
    args.extend(["-map", "0:a:0", "-vn", "-map_metadata", "-1"].map(String::from));
    match plan.format {
        Format::Mp3 => args.extend(["-c:a", "libmp3lame", "-f", "mp3"].map(String::from)),
        Format::Aac => args.extend(["-c:a", "aac", "-f", "adts"].map(String::from)),
    }
    args.extend(["-b:a".into(), format!("{}k", plan.kbps), "pipe:1".into()]);
    args
}

/// Starts the transcode of `source`. `None` when every slot is taken or ffmpeg
/// can't be started; the caller then sends the original.
pub fn start(source: Body, plan: Plan) -> Option<Response> {
    let permit = SLOTS.try_acquire().ok()?;
    let mut child = Command::new("ffmpeg")
        .args(args(plan))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A listener who skips ahead drops the response; the process goes with it.
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| tracing::warn!(error = %err, "ffmpeg could not be started for a transcode"))
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;

    tokio::spawn(async move {
        let mut input = source.into_data_stream();
        while let Some(chunk) = input.next().await {
            // A write error means ffmpeg has stopped (the client went away).
            let Ok(chunk) = chunk else { break };
            if stdin.write_all(&chunk).await.is_err() {
                break;
            }
        }
        // Closing stdin tells ffmpeg the input is complete.
        drop(stdin);
    });

    // The slot is held for as long as the process lives.
    tokio::spawn(async move {
        let _permit = permit;
        let _ = child.wait().await;
    });

    let body = Body::from_stream(tokio_util::io::ReaderStream::new(stdout));
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
        let a = args(Plan { format: Format::Mp3, kbps: 128, offset_secs: 0 });
        assert!(a.windows(2).any(|w| w == ["-c:a", "libmp3lame"]));
        assert!(a.windows(2).any(|w| w == ["-b:a", "128k"]));
        assert!(!a.contains(&"-ss".to_string()));
        let b = args(Plan { format: Format::Aac, kbps: 96, offset_secs: 45 });
        assert!(b.windows(2).any(|w| w == ["-ss", "45"]));
        assert!(b.windows(2).any(|w| w == ["-f", "adts"]));
    }
}
