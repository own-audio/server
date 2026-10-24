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

/// Why a song of `duration` seconds is not worth fingerprinting, if it is not.
fn too_short(duration: Option<i32>) -> Option<String> {
    duration
        .filter(|d| *d < MIN_SECS)
        .map(|d| format!("only {d} s of audio: too short to fingerprint (a damaged or cut-off file?)"))
}

/// The ffmpeg arguments for one fingerprint of `input` (a path or a link):
/// mono, the first two minutes, Chromaprint compressed and base64 to stdout.
fn ffmpeg_args(input: &str) -> Vec<String> {
    [
        "-hide_banner", "-nostdin", "-loglevel", "error",
        "-i", input,
        "-t", SECONDS,
        "-ac", "1",
        "-f", "chromaprint", "-fp_format", "base64", "-",
    ]
    .map(String::from)
    .to_vec()
}

/// ffmpeg's error, short, without the query string of a link it was given
/// (the signature is what makes it a credential).
fn reason(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("ffmpeg failed");
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
    let Some(source) = db::fingerprints::source(pool, track_id).await? else {
        // Deleted or trashed since it was queued.
        return Ok(());
    };

    if let Some(why) = too_short(source.duration_secs) {
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
    if out.status.success() && !fingerprint.is_empty() {
        db::fingerprints::save(pool, track_id, &source, Some(&fingerprint), None).await
    } else {
        let why = if out.status.success() { "no audio to fingerprint".to_string() } else { reason(&out.stderr) };
        tracing::info!(%track_id, reason = %why, "no fingerprint: the file does not decode");
        db::fingerprints::save(pool, track_id, &source, None, Some(&why)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_the_first_two_minutes_in_mono_for_acoustid() {
        let args = ffmpeg_args("/music/a.flac");
        let joined = args.join(" ");
        assert!(joined.contains("-i /music/a.flac -t 120 -ac 1 -f chromaprint -fp_format base64 -"), "{joined}");
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
    fn a_reason_never_carries_a_link_signature() {
        let err = b"https://r2.example/bucket/k.mp3?X-Amz-Signature=secret: Invalid data found when processing input\n";
        let why = reason(err);
        assert!(!why.contains("secret"), "{why}");
        assert!(why.contains("Invalid data found"), "{why}");
    }
}
