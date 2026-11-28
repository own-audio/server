// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acoustic fingerprints (Chromaprint) of songs, made by ffmpeg — the image's
//! ffmpeg is built with Chromaprint, the library AcoustID's own data comes
//! from, so a fingerprint here is one AcoustID recognises. The first two
//! minutes are enough, as for AcoustID; on object storage only those are read.

use crate::db;
use crate::storage::ObjectStore;
use anyhow::Context;
use sqlx::PgPool;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

pub const JOB_TYPE: &str = "fingerprint_track";

/// AcoustID fingerprints the first 120 seconds of a recording.
const SECONDS: &str = "120";
/// A song that has not decoded in this long is not going to.
const TIMEOUT: Duration = Duration::from_secs(120);
/// How long a link to an object in S3 stays valid for ffmpeg to read it.
const LINK_SECS: u64 = 600;
/// Shorter than this, a fingerprint identifies nothing: on the NAS test, the
/// songs this short were cut-off downloads (0.3 s and 6 s of a 3-minute song).
const MIN_SECS: i32 = 10;

/// A song this short is not a song: a download that stopped after its first
/// frames, a placeholder. It is quarantined, not only left unfingerprinted.
const STUB_SECS: i32 = 2;
/// Nothing above this in the first two minutes is silence: a DRM-protected WMA
/// decodes to exact zeros, and a player would play two minutes of nothing.
const SILENCE_DB: f64 = -80.0;

/// Whether this machine's ffmpeg has the Chromaprint muxer, asked once. Images
/// built from `backend/Dockerfile` have it; a binary run outside the image may
/// find an ffmpeg without it (Homebrew's, for one), and then every song would
/// fail the same way — no fingerprints rather than a quarantined library.
pub async fn available() -> bool {
    static AVAILABLE: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    *AVAILABLE
        .get_or_init(|| async {
            let probe = Command::new("ffmpeg")
                .args(["-hide_banner", "-nostdin", "-muxers"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .output();
            let found = match tokio::time::timeout(Duration::from_secs(10), probe).await {
                Ok(Ok(out)) => lists_chromaprint(&out.stdout),
                _ => false,
            };
            if !found {
                tracing::warn!(
                    "ffmpeg has no Chromaprint (the `chromaprint` muxer): acoustic fingerprints are off, \
                     as with FINGERPRINT__MODE=off. The server's image includes it; elsewhere, use an \
                     ffmpeg built with --enable-chromaprint"
                );
            }
            found
        })
        .await
}

/// Whether `ffmpeg -muxers` lists `chromaprint` (lines like `  E  chromaprint  Chromaprint`).
fn lists_chromaprint(muxers: &[u8]) -> bool {
    String::from_utf8_lossy(muxers).lines().any(|l| l.split_whitespace().nth(1) == Some("chromaprint"))
}

/// Whether ffmpeg failed on its own command line, not on the file: no
/// Chromaprint muxer, so `-fp_format` is an unknown option or `chromaprint` an
/// unknown format. Such a failure says nothing about the song.
fn chromaprint_unavailable(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr);
    text.contains("Error splitting the argument list")
        || text.contains("Unrecognized option 'fp_format'")
        || text.contains("Unknown format 'chromaprint'")
        || text.contains("Requested output format 'chromaprint'")
}

/// Why a song of `duration` seconds is not worth fingerprinting, if it is not.
fn too_short(duration: Option<i32>) -> Option<String> {
    duration
        .filter(|d| *d < MIN_SECS)
        .map(|d| format!("only {d} s of audio: too short to fingerprint (a damaged or cut-off file?)"))
}

/// Why a song of `duration` seconds is quarantined, if it is: a stub.
fn stub(duration: Option<i32>) -> Option<String> {
    duration.filter(|d| *d < STUB_SECS).map(|d| format!("only {d} s of audio: a stub, not a song"))
}

/// The ffmpeg arguments for one fingerprint of `input` (a path or a link):
/// mono, the first two minutes, Chromaprint compressed and base64 to stdout.
/// The loudness of the same two minutes goes to stderr (`volumedetect`, at
/// info level), so a silent file is known at the same cost.
fn ffmpeg_args(input: &str) -> Vec<String> {
    [
        "-hide_banner", "-nostdin", "-loglevel", "info",
        "-i", input,
        "-t", SECONDS,
        "-ac", "1",
        "-af", "volumedetect",
        "-f", "chromaprint", "-fp_format", "base64", "-",
    ]
    .map(String::from)
    .to_vec()
}

/// How much audio ffmpeg decoded: its last progress line's `time=`. A download
/// that stopped after its first frames still carries the whole song's headers
/// (a Xing frame says 30 s of a file holding 0.2 s), so the length on record
/// cannot tell; the decode can.
fn decoded_seconds(stderr: &[u8]) -> Option<f64> {
    let text = String::from_utf8_lossy(stderr);
    let stamp = text.rmatch_indices("time=").map(|(i, _)| &text[i + 5..]).find_map(|rest| rest.split_whitespace().next())?;
    let mut parts = stamp.split(':').rev();
    let secs: f64 = parts.next()?.parse().ok()?;
    let mins: f64 = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0.0);
    let hours: f64 = parts.next().and_then(|h| h.parse().ok()).unwrap_or(0.0);
    Some(hours * 3600.0 + mins * 60.0 + secs).filter(|t| t.is_finite())
}

/// Why a song whose headers promise `recorded` seconds but that decodes to
/// `decoded` is set aside, if it is: a stub, or a download cut off early. The
/// fingerprint decodes the first two minutes at most, so a longer song is
/// judged on those.
fn cut_off(recorded: Option<i32>, decoded: f64) -> Option<String> {
    if decoded < STUB_SECS as f64 {
        return Some(format!("only {decoded:.1} s of audio decodes: a stub, not a song"));
    }
    let expected = recorded.filter(|r| *r >= MIN_SECS)? .min(SECONDS.parse::<i32>().unwrap_or(120)) as f64;
    (decoded < expected * 0.5).then(|| format!("only {decoded:.0} s of {expected:.0} s decode: a download cut off early"))
}

/// The loudest sample of the decoded audio, from `volumedetect`'s report.
fn max_volume_db(stderr: &[u8]) -> Option<f64> {
    let text = String::from_utf8_lossy(stderr);
    text.lines().find_map(|l| l.split("max_volume:").nth(1)?.split_whitespace().next()?.parse::<f64>().ok())
}

/// ffmpeg's error, short, without the query string of a link it was given
/// (the signature is what makes it a credential). At info level stderr also
/// carries the input's description and progress; the error is the line that
/// says so, else the last line.
fn reason(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("[Parsed_volumedetect")).collect();
    let line = lines
        .iter()
        .copied()
        .find(|l| ["rror", "Invalid", "Failed", "failed", "Could not", "could not", "not found"].iter().any(|w| l.contains(w)))
        .or_else(|| lines.last().copied())
        .unwrap_or("ffmpeg failed");
    let redacted: Vec<String> = line
        .split_whitespace()
        .map(|w| match w.split_once('?') {
            Some((before, _)) if w.contains("://") => format!("{before}?…"),
            _ => w.to_string(),
        })
        .collect();
    redacted.join(" ").chars().take(300).collect()
}

/// Fingerprint one song. A file ffmpeg cannot decode is a result, not a job
/// failure: its row records why, and it is not tried again until it changes.
pub async fn execute(pool: &PgPool, storage: &ObjectStore, job: &crate::jobs::models::Job) -> anyhow::Result<()> {
    let track_id: Uuid = job
        .payload
        .as_ref()
        .and_then(|p| p.get("track_id"))
        .and_then(|v| v.as_str())
        .context("missing track_id in payload")?
        .parse()?;
    if !available().await {
        // Queued before the probe said no; leave the song unfingerprinted and untouched.
        return Ok(());
    }
    let Some(source) = db::fingerprints::source(pool, track_id).await? else {
        // Deleted or trashed since it was queued.
        return Ok(());
    };

    if let Some(why) = too_short(source.duration_secs) {
        if let Some(stub) = stub(source.duration_secs) {
            set_aside(pool, track_id, &stub).await;
        }
        return db::fingerprints::save(pool, track_id, &source, None, Some(&why)).await;
    }

    let input = match storage.local_file(&source.object_key) {
        Some(path) => path?.to_string_lossy().into_owned(),
        None => storage.internal_get_url(&source.object_key, LINK_SECS).await?,
    };
    let run = Command::new("ffmpeg")
        .args(ffmpeg_args(&input))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, run).await.context("ffmpeg timed out")?.context("could not run ffmpeg")?;

    let fingerprint = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() && chromaprint_unavailable(&out.stderr) {
        anyhow::bail!("ffmpeg cannot fingerprint (no Chromaprint): {}", reason(&out.stderr));
    }
    if out.status.success() && !fingerprint.is_empty() {
        if let Some(why) = decoded_seconds(&out.stderr).and_then(|d| cut_off(source.duration_secs, d)) {
            set_aside(pool, track_id, &why).await;
        } else if let Some(db) = max_volume_db(&out.stderr).filter(|db| *db <= SILENCE_DB) {
            set_aside(pool, track_id, &format!("silent: nothing above {db} dB in its first two minutes (a DRM-protected file?)")).await;
        }
        db::fingerprints::save(pool, track_id, &source, Some(&fingerprint), None).await
    } else {
        let why = if out.status.success() { "no audio to fingerprint".to_string() } else { reason(&out.stderr) };
        tracing::info!(%track_id, reason = %why, "no fingerprint: the file does not decode");
        set_aside(pool, track_id, &format!("does not decode: {why}")).await;
        db::fingerprints::save(pool, track_id, &source, None, Some(&why)).await
    }
}

/// Into quarantine (migration 0124), with why; a failure to record it is logged, not fatal.
async fn set_aside(pool: &PgPool, track_id: Uuid, why: &str) {
    match db::quarantine::quarantine(pool, track_id, why).await {
        Ok(true) => tracing::warn!(%track_id, reason = %why, "quarantined: the file is broken"),
        Ok(false) => {}
        Err(e) => tracing::warn!(%track_id, error = %format!("{e:#}"), "not quarantined"),
    }
}

/// Two fingerprints at least this alike are one recording. Measured 2026-11-28 on
/// the NAS library (10,604 fingerprints): one recording in two files, lengths within
/// 2 s, scored 0.84–1.0 (median 0.95); two other songs of the same length 0.47–0.61.
pub const SAME_RECORDING: f32 = 0.80;

/// A fingerprint as Chromaprint's 32-bit frames, from the compressed base64 form
/// it is stored in (`fingerprint_decompressor.cpp`): a 4-byte header (algorithm,
/// frame count), each frame's set bits as gaps packed in 3 bits, gaps of 7 and more
/// finished in a 5-bit section, each frame XORed with the one before.
pub fn decode(fingerprint: &str) -> Option<Vec<u32>> {
    use base64::Engine;
    let data = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(fingerprint.trim_end_matches('=')).ok()?;
    if data.len() < 4 {
        return None;
    }
    let frames = (usize::from(data[1]) << 16) | (usize::from(data[2]) << 8) | usize::from(data[3]);
    let rest = &data[4..];
    let mut gaps = Vec::new();
    let mut ends = 0;
    for v in unpack(rest, 3) {
        if ends == frames {
            break;
        }
        gaps.push(v);
        if v == 0 {
            ends += 1;
        }
    }
    if ends != frames {
        return None;
    }
    let mut exceptions = unpack(&rest[(gaps.len() * 3).div_ceil(8)..], 5).into_iter();
    let mut out = Vec::with_capacity(frames);
    let (mut value, mut bit) = (0u32, 0u32);
    for g in gaps {
        let g = if g == 7 { 7 + exceptions.next()? } else { g };
        if g == 0 {
            out.push(out.last().map_or(value, |prev| value ^ prev));
            (value, bit) = (0, 0);
        } else {
            bit += g;
            if bit > 32 {
                return None;
            }
            value |= 1 << (bit - 1);
        }
    }
    Some(out)
}

/// Little-endian bit stream of `width`-bit values.
fn unpack(bytes: &[u8], width: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(bytes.len() * 8 / width as usize);
    let (mut acc, mut held) = (0u64, 0u32);
    for &b in bytes {
        acc |= u64::from(b) << held;
        held += 8;
        while held >= width {
            out.push((acc & ((1 << width) - 1)) as u32);
            acc >>= width;
            held -= width;
        }
    }
    out
}

/// The share of bits two fingerprints have in common at their best alignment, up
/// to about ten seconds apart (a rip with a longer lead-in): 1 for one file, ~0.5
/// for unrelated songs. 0 when they overlap too little to say.
pub fn similarity(a: &[u32], b: &[u32]) -> f32 {
    // Chromaprint makes about eight frames a second.
    const SHIFT: isize = 80;
    const OVERLAP: usize = 100;
    let mut best = 0.0f32;
    for offset in -SHIFT..=SHIFT {
        let (x, y) = if offset >= 0 { (a.get(offset as usize..), Some(b)) } else { (Some(a), b.get((-offset) as usize..)) };
        let (Some(x), Some(y)) = (x, y) else { continue };
        let n = x.len().min(y.len());
        if n < OVERLAP {
            continue;
        }
        let differ: u32 = x.iter().zip(y).take(n).map(|(p, q)| (p ^ q).count_ones()).sum();
        best = best.max(1.0 - differ as f32 / (32 * n) as f32);
    }
    best
}

#[cfg(test)]
mod tests {

    /// Chromaprint's compression, the other way round, to make fixtures.
    fn encode(frames: &[u32]) -> String {
        use base64::Engine;
        let (mut gaps, mut exceptions) = (Vec::new(), Vec::new());
        let mut prev = 0;
        for &f in frames {
            let x = f ^ prev;
            prev = f;
            let mut last = 0;
            for bit in 1..=32u32 {
                if x & (1 << (bit - 1)) != 0 {
                    let g = bit - last;
                    last = bit;
                    if g >= 7 {
                        gaps.push(7);
                        exceptions.push(g - 7);
                    } else {
                        gaps.push(g);
                    }
                }
            }
            gaps.push(0);
        }
        let pack = |vals: &[u32], width: u32| {
            let (mut out, mut acc, mut held) = (Vec::new(), 0u64, 0u32);
            for &v in vals {
                acc |= u64::from(v) << held;
                held += width;
                while held >= 8 {
                    out.push(acc as u8);
                    acc >>= 8;
                    held -= 8;
                }
            }
            if held > 0 {
                out.push(acc as u8);
            }
            out
        };
        let n = frames.len();
        let mut data = vec![1, (n >> 16) as u8, (n >> 8) as u8, n as u8];
        data.extend(pack(&gaps, 3));
        data.extend(pack(&exceptions, 5));
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
    }

    fn noise(seed: u64, n: usize) -> Vec<u32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u32
            })
            .collect()
    }

    #[test]
    fn a_fingerprint_decodes_to_its_frames() {
        let frames = noise(7, 948);
        assert_eq!(decode(&encode(&frames)), Some(frames));
        assert_eq!(decode("AQAD"), None, "a header without frames");
        assert_eq!(decode("not base64!"), None);
    }

    #[test]
    fn one_recording_matches_and_another_does_not() {
        let a = noise(1, 948);
        // The same audio a second later in the file, with a few bits flipped.
        let mut b: Vec<u32> = a[8..].to_vec();
        for (i, f) in b.iter_mut().enumerate() {
            if i % 3 == 0 {
                *f ^= 0b1011;
            }
        }
        assert!(similarity(&a, &b) > SAME_RECORDING, "{}", similarity(&a, &b));
        let other = noise(2, 948);
        let s = similarity(&a, &other);
        assert!(s < 0.65 && s > 0.4, "{s}");
        assert_eq!(similarity(&a[..50], &a[..50]), 0.0, "too short to say");
    }

    use super::*;

    #[test]
    fn fingerprints_the_first_two_minutes_in_mono_for_acoustid() {
        let args = ffmpeg_args("/music/a.flac");
        let joined = args.join(" ");
        assert!(joined.contains("-i /music/a.flac -t 120 -ac 1 -af volumedetect -f chromaprint -fp_format base64 -"), "{joined}");
    }

    #[test]
    fn the_loudness_report_and_the_error_are_read_from_a_chatty_stderr() {
        let stderr = b"Input #0, mp3, from 'x.mp3':\n  Duration: 00:03:41.09\n[Parsed_volumedetect_0 @ 0x1] n_samples: 1000\n[Parsed_volumedetect_0 @ 0x1] mean_volume: -91.0 dB\n[Parsed_volumedetect_0 @ 0x1] max_volume: -91.0 dB\nsize=N/A time=00:02:00.00\n";
        assert_eq!(max_volume_db(stderr), Some(-91.0));
        let failing = b"Input #0, mp3, from 'x.mp3':\n[mp3 @ 0x1] Failed to read frame size: Could not seek to 1151.\nx.mp3: Invalid data found when processing input\n";
        assert_eq!(reason(failing), "[mp3 @ 0x1] Failed to read frame size: Could not seek to 1151.");
        assert_eq!(max_volume_db(failing), None);
    }

    #[test]
    fn a_file_that_decodes_far_shorter_than_its_headers_say_is_cut_off() {
        let progress = b"Input #0\nsize=N/A time=00:00:00.18 bitrate=N/A speed=10x\n[Parsed_volumedetect_0 @ 0x1] max_volume: -3.0 dB\n";
        assert_eq!(decoded_seconds(progress), Some(0.18));
        assert!(cut_off(Some(30), 0.18).unwrap().contains("a stub"), "4 KB of a 30 s song with the song's own Xing header");
        assert!(cut_off(Some(200), 40.0).unwrap().contains("cut off"), "40 s of 120 s decoded");
        assert_eq!(cut_off(Some(200), 119.5), None, "the two minutes the fingerprint reads, in full");
        assert_eq!(cut_off(Some(30), 29.0), None);
        assert_eq!(cut_off(None, 25.0), None, "no length on record: nothing to compare");
        assert_eq!(decoded_seconds(b"time=01:02:03.50 "), Some(3723.5));
    }

    #[test]
    fn a_stub_is_a_second_or_two_a_short_song_is_not() {
        assert!(stub(Some(0)).is_some());
        assert!(stub(Some(1)).is_some());
        assert!(stub(Some(4)).is_none(), "a 4 s jingle is a song");
        assert!(stub(None).is_none());
    }

    #[test]
    fn a_few_seconds_of_audio_is_not_fingerprinted() {
        assert!(too_short(Some(0)).is_some_and(|w| w.starts_with("only 0 s")));
        assert!(too_short(Some(5)).is_some());
        assert!(too_short(Some(10)).is_none());
        assert!(too_short(Some(240)).is_none());
        // A length nobody could read is tried: ffmpeg finds out.
        assert!(too_short(None).is_none());
    }

    #[test]
    fn an_ffmpeg_without_chromaprint_is_not_a_broken_file() {
        // Homebrew's ffmpeg 8 (no --enable-chromaprint), with the job's arguments:
        assert!(chromaprint_unavailable(b"Unrecognized option 'fp_format'.\nError splitting the argument list: Option not found\n"));
        assert!(chromaprint_unavailable(b"[out#0 @ 0x1] Unknown format 'chromaprint'.\n"));
        assert!(chromaprint_unavailable(b"Requested output format 'chromaprint' is not known.\n"));
        // A file that does not decode still is one.
        assert!(!chromaprint_unavailable(b"x.mp3: Invalid data found when processing input\n"));
        assert!(!chromaprint_unavailable(b"[mp3 @ 0x1] Failed to read frame size: Could not seek to 1151.\n"));

        assert!(lists_chromaprint(b"Formats:\n .E. = Muxing supported\n ---\n  E  caf             Apple CAF\n  E  chromaprint     Chromaprint\n"));
        assert!(!lists_chromaprint(b"Formats:\n ---\n  E  caf             Apple CAF (Core Audio Format)\n"));
    }

    #[test]
    fn a_reason_never_carries_a_link_signature() {
        let err = b"https://r2.example/bucket/k.mp3?X-Amz-Signature=secret: Invalid data found when processing input\n";
        let why = reason(err);
        assert!(!why.contains("secret"), "{why}");
        assert!(why.contains("Invalid data found"), "{why}");
    }
}
