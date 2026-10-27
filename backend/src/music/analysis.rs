// SPDX-License-Identifier: AGPL-3.0-or-later
//! Measuring what a track actually sounds like.
//!
//! See `docs/music-signals-and-smart-playlists-plan.md` §3. All of this runs
//! server-side, on one extractor, so every value in the library is comparable —
//! which is what makes the percentile in S4 mean anything.
//!
//! **This module measures. It does not decide.** No "energy" is computed here:
//! energy is a percentile across the whole library, so it belongs in SQL over
//! these columns, not in a per-track function that cannot see the others.
//!
//! Licensing (§3.7): `ffmpeg` is LGPL and `aubio` is GPL-3. Neither is
//! distributed — the backend is sold as a service and never conveyed — so both
//! are free to use here. Essentia is AGPL-3, which *is* triggered by network
//! use, and is therefore not an option without a commercial licence.

use anyhow::{Context, bail};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Bump when the extraction *formula* changes, so affected rows can be
/// recomputed. The goal is never bumping this to re-fetch data — that is why
/// `analysis_raw` keeps everything the extractor produced.
pub const ANALYSIS_VERSION: i16 = 1;

/// The decode contract. Every measurement is taken from audio in exactly this
/// shape, so the same file gives the same numbers regardless of container,
/// codec or which machine ran it. Borrowed from `bliss-rs`, which is right
/// about this even though its opaque feature vector was the wrong tool for us.
pub const SAMPLE_RATE: u32 = 22_050;

/// Seconds taken from the middle of the track. Enough for tempo and loudness,
/// and it is the difference between fetching ~1.5 MB and ~10 MB per track when
/// the object lives in R2.
pub const WINDOW_SECS: u32 = 60;

/// How long one track may take before we give up and leave it unmeasured. A
/// pathological file must not wedge the worker.
const TIMEOUT_SECS: u64 = 90;

const FFT_SIZE: usize = 2048;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackAnalysis {
    pub bpm: Option<f32>,
    pub music_key: Option<String>,
    pub key_scale: Option<String>,
    pub loudness_lufs: Option<f32>,
    pub dynamic_range: Option<f32>,
    pub spectral_centroid: Option<f32>,
    pub onset_rate: Option<f32>,
}

/// Measure one audio file.
///
/// Errors only for conditions worth retrying (a missing binary, a timeout). A
/// file that simply will not decode, or has no detectable beat, comes back with
/// `None` in the fields that could not be measured — that is a fact about the
/// track, not a failure, and the caller stores it rather than retrying forever.
pub async fn analyze(path: &Path) -> anyhow::Result<TrackAnalysis> {
    let pcm = decode_window(path).await?;
    if pcm.len() < SAMPLE_RATE as usize {
        // Under a second of audio: every measurement below would be noise.
        return Ok(TrackAnalysis::default());
    }

    let (centroid, onset_rate) = spectral_features(&pcm);
    let (peak, rms) = level(&pcm);

    Ok(TrackAnalysis {
        bpm: tempo(path).await.unwrap_or(None),
        music_key: None,
        key_scale: None,
        loudness_lufs: loudness(path).await.unwrap_or(None),
        dynamic_range: if rms > 0.0 && peak > 0.0 {
            Some(20.0 * (peak / rms).log10())
        } else {
            None
        },
        spectral_centroid: Some(centroid),
        onset_rate: Some(onset_rate),
    })
}

/// Decode `WINDOW_SECS` from the middle of the file to mono f32 at
/// `SAMPLE_RATE`.
///
/// Taken from the middle rather than the start because intros are not
/// representative — a track that opens with ten seconds of silence or a spoken
/// sample would otherwise be measured on that.
async fn decode_window(path: &Path) -> anyhow::Result<Vec<f32>> {
    let duration = probe_duration(path).await.unwrap_or(None);
    let start = duration
        .map(|d| ((d - WINDOW_SECS as f64) / 2.0).max(0.0))
        .unwrap_or(0.0);

    let out = run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-ss",
            &format!("{start:.3}"),
            "-t",
            &WINDOW_SECS.to_string(),
            "-i",
            &path.to_string_lossy(),
            "-vn",
            "-ac",
            "1",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-f",
            "f32le",
            "-",
        ],
    )
    .await?;

    if !out.status.success() {
        bail!(
            "ffmpeg could not decode {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

async fn probe_duration(path: &Path) -> anyhow::Result<Option<f64>> {
    let out = run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            &path.to_string_lossy(),
        ],
    )
    .await?;

    Ok(String::from_utf8_lossy(&out.stdout).trim().parse().ok())
}

/// EBU R128 integrated loudness, from ffmpeg's own filter. An exact
/// measurement — the summary is printed to stderr, which is why it is parsed
/// rather than read from stdout.
async fn loudness(path: &Path) -> anyhow::Result<Option<f32>> {
    let out = run(
        "ffmpeg",
        &[
            "-v",
            "info",
            "-i",
            &path.to_string_lossy(),
            "-af",
            // NOT `ebur128=framelog=quiet`. ffmpeg only learned that option in
            // 6.x; on 5.1 (Debian bookworm's, which the image shipped until it built its own) the
            // filter fails to initialise and the summary reports a flat
            // 0.0 LUFS. That is the worst kind of bug — it measures the whole
            // library successfully and every value is wrong.
            //
            // Plain `ebur128` logs a line per 100 ms, which is noise we
            // discard, and prints the same summary on every version.
            "ebur128",
            "-f",
            "null",
            "-",
        ],
    )
    .await?;

    Ok(parse_lufs(&String::from_utf8_lossy(&out.stderr)))
}

/// Pull integrated loudness out of ffmpeg's ebur128 summary.
///
/// Pure so it can be tested without ffmpeg installed — the summary goes to
/// stderr in a human-readable block, and its shape is the sort of thing that
/// changes quietly between ffmpeg releases.
fn parse_lufs(stderr: &str) -> Option<f32> {
    stderr
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("I:"))
        .and_then(|v| v.trim().trim_end_matches("LUFS").trim().parse::<f32>().ok())
        // A failed filter reports exactly 0.0, and no real recording measures
        // that — it would be a full-scale square wave. Treating it as "not
        // measured" is what stops a broken toolchain from quietly filling the
        // library with a plausible-looking constant.
        .filter(|v| *v < -1.0)
}

/// Tempo via aubio's beat tracker. `None` rather than an error when no beat is
/// detectable — ambient and rubato classical genuinely have no BPM, and
/// recording that fact is more useful than retrying forever.
///
/// The image's aubio reads only WAV (it is built without decoders of its own),
/// so ffmpeg decodes the whole track to a mono 16-bit WAV first, in the temp
/// directory, removed when this returns. The file's own sample rate is kept:
/// the beat tracker's frames are counted in samples, and at the file's rate it
/// finds the beats `aubio tempo` found before (resampled to 44.1 kHz, a 48 kHz
/// track measured 128.12 instead of 128.24).
async fn tempo(path: &Path) -> anyhow::Result<Option<f32>> {
    let wav = tempfile::Builder::new()
        .prefix("tempo-")
        .suffix(".wav")
        .tempfile()
        .context("create a temp file for the beat tracker")?;
    let wav_path = wav.path().to_string_lossy().into_owned();
    let decoded = run(
        "ffmpeg",
        &[
            "-v", "error", "-y",
            "-i", &path.to_string_lossy(),
            "-vn", "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav",
            &wav_path,
        ],
    )
    .await?;
    if !decoded.status.success() {
        return Ok(None);
    }

    // The frame sizes `aubio tempo` used.
    let out = run("aubiotrack", &["-i", &wav_path, "-B", "1024", "-H", "512"]).await?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(parse_tempo(&String::from_utf8_lossy(&out.stdout)))
}

/// The BPM from `aubiotrack`'s beats, one time in seconds per line, worked out
/// as `aubio tempo` did: the mean of the beat-to-beat tempi. Under ten beats is
/// what aubio called "uncertain" and stays no tempo. Values outside 20–250 are
/// detector noise rather than music, and are dropped — an unmeasurable tempo is
/// a fact about the track, and recording `None` beats recording a wrong number
/// that then filters playlists.
fn parse_tempo(stdout: &str) -> Option<f32> {
    let beats: Vec<f64> = stdout.lines().filter_map(|l| l.trim().parse::<f64>().ok()).collect();
    if beats.len() < 10 {
        return None;
    }
    let tempi: Vec<f64> = beats.windows(2).map(|w| 60.0 / (w[1] - w[0])).collect();
    let bpm = tempi.iter().sum::<f64>() / tempi.len() as f64;
    Some(bpm as f32).filter(|b| (20.0..=250.0).contains(b))
}

/// Peak and RMS amplitude.
fn level(pcm: &[f32]) -> (f32, f32) {
    let peak = pcm.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    let sum_sq: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let rms = (sum_sq / pcm.len() as f64).sqrt() as f32;
    (peak, rms)
}

/// Spectral centroid (Hz) and onset rate (per second), over Hann-windowed
/// frames.
///
/// The centroid is the brightness axis and the onset rate is the busyness
/// axis; together with loudness and dynamic range they are what S4's energy
/// percentile is built from. Onsets are counted by spectral flux rising above
/// a running mean — crude next to a dedicated detector, but it only has to
/// rank tracks against each other consistently, not be correct in absolute
/// terms.
fn spectral_features(pcm: &[f32]) -> (f32, f32) {
    let hop = FFT_SIZE / 2;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos()
        })
        .collect();

    let bin_hz = SAMPLE_RATE as f32 / FFT_SIZE as f32;
    let mut centroid_sum = 0.0_f64;
    let mut centroid_frames = 0_u32;
    let mut flux = Vec::new();
    let mut prev: Vec<f32> = vec![0.0; FFT_SIZE / 2];

    let mut buf = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
    for frame in pcm.windows(FFT_SIZE).step_by(hop) {
        for (i, c) in buf.iter_mut().enumerate() {
            *c = Complex32::new(frame[i] * window[i], 0.0);
        }
        fft.process(&mut buf);

        let mag: Vec<f32> = buf[..FFT_SIZE / 2].iter().map(|c| c.norm()).collect();
        let total: f32 = mag.iter().sum();
        if total > 1e-6 {
            let weighted: f32 = mag
                .iter()
                .enumerate()
                .map(|(i, m)| i as f32 * bin_hz * m)
                .sum();
            centroid_sum += (weighted / total) as f64;
            centroid_frames += 1;
        }

        // Positive spectral flux: only increases count as an onset.
        flux.push(
            mag.iter()
                .zip(prev.iter())
                .map(|(m, p)| (m - p).max(0.0))
                .sum::<f32>(),
        );
        prev = mag;
    }

    let centroid = if centroid_frames > 0 {
        (centroid_sum / centroid_frames as f64) as f32
    } else {
        0.0
    };

    let onsets = count_onsets(&flux);
    let seconds = pcm.len() as f32 / SAMPLE_RATE as f32;
    let onset_rate = if seconds > 0.0 {
        onsets as f32 / seconds
    } else {
        0.0
    };

    (centroid, onset_rate)
}

/// Peaks in the flux curve that exceed a local mean, counted once per rise.
fn count_onsets(flux: &[f32]) -> u32 {
    if flux.is_empty() {
        return 0;
    }
    let mean: f32 = flux.iter().sum::<f32>() / flux.len() as f32;
    let threshold = mean * 1.5;

    let mut count = 0_u32;
    let mut above = false;
    for f in flux {
        if *f > threshold {
            if !above {
                count += 1;
                above = true;
            }
        } else {
            above = false;
        }
    }
    count
}

async fn run(program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let fut = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    tokio::time::timeout(std::time::Duration::from_secs(TIMEOUT_SECS), fut)
        .await
        .with_context(|| format!("{program} timed out after {TIMEOUT_SECS}s"))?
        .with_context(|| format!("could not run {program}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_no_dynamic_range_and_no_onsets() {
        let pcm = vec![0.0_f32; SAMPLE_RATE as usize * 2];
        let (peak, rms) = level(&pcm);
        assert_eq!(peak, 0.0);
        assert_eq!(rms, 0.0);
        let (centroid, onset_rate) = spectral_features(&pcm);
        assert_eq!(centroid, 0.0);
        assert_eq!(onset_rate, 0.0);
    }

    /// The centroid is the brightness axis, so a high tone must land far above
    /// a low one. Exact values do not matter — only that the ordering holds,
    /// because energy is a percentile over the library, not an absolute.
    #[test]
    fn a_bright_tone_has_a_higher_centroid_than_a_dark_one() {
        let tone = |hz: f32| -> Vec<f32> {
            (0..SAMPLE_RATE as usize * 2)
                .map(|i| {
                    (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE as f32).sin() * 0.5
                })
                .collect()
        };
        let (low, _) = spectral_features(&tone(220.0));
        let (high, _) = spectral_features(&tone(4000.0));
        assert!(high > low * 2.0, "low {low} Hz vs high {high} Hz");
    }

    #[test]
    fn a_steady_tone_is_quieter_in_dynamic_range_than_a_pulse() {
        let steady: Vec<f32> = (0..10_000).map(|_| 0.5).collect();
        let mut pulse = vec![0.01_f32; 10_000];
        pulse[5_000] = 1.0;

        let (p1, r1) = level(&steady);
        let (p2, r2) = level(&pulse);
        let dr = |p: f32, r: f32| 20.0 * (p / r).log10();
        assert!(dr(p2, r2) > dr(p1, r1));
    }

    #[test]
    fn onsets_are_counted_once_per_rise() {
        // Two sustained excursions above the threshold, not four.
        let flux = vec![0.0, 0.0, 10.0, 10.0, 0.0, 0.0, 10.0, 10.0, 0.0];
        assert_eq!(count_onsets(&flux), 2);
    }

    /// Captured from ffmpeg 8. The summary is human-readable output on stderr,
    /// so its shape can change between releases — if this breaks, loudness is
    /// silently `None` for the whole library rather than failing loudly.
    #[test]
    fn integrated_loudness_is_read_from_the_ebur128_summary() {
        let stderr = "\
[Parsed_ebur128_0 @ 0x77070249c0] Summary:

  Integrated loudness:
    I:         -21.8 LUFS
    Threshold: -31.7 LUFS

  Loudness range:
    LRA:         0.0 LU
    Threshold: -41.7 LUFS
    LRA low:   -21.8 LUFS
    LRA high:  -21.8 LUFS
";
        assert_eq!(parse_lufs(stderr), Some(-21.8));
        assert_eq!(parse_lufs("no summary here"), None);
        assert_eq!(parse_lufs(""), None);
    }

    /// Regression. `ebur128=framelog=quiet` is an ffmpeg 6+ option; on 5.1
    /// (Debian bookworm's) the filter fails to initialise and still prints a
    /// summary — of 0.0 LUFS. The first run against a real track measured the
    /// whole library "successfully" with that constant. No real recording is
    /// 0.0 LUFS, so it is rejected rather than stored.
    #[test]
    fn a_failed_ebur128_filter_reports_zero_and_is_not_believed() {
        let broken = "\
[AVFilterGraph @ 0x1] Error initializing filter 'ebur128' with args 'framelog=quiet'
  Integrated loudness:
    I:           0.0 LUFS
    Threshold:   0.0 LUFS
";
        assert_eq!(parse_lufs(broken), None);
    }

    /// `count` beat times, `step` seconds apart, as aubiotrack prints them.
    fn beats(count: usize, step: f64) -> String {
        (0..count).map(|i| format!("{:.6}\n", 0.023 + i as f64 * step)).collect()
    }

    #[test]
    fn tempo_is_the_mean_of_the_beat_intervals() {
        // 0.5 s apart is 120 BPM.
        let bpm = parse_tempo(&beats(12, 0.5)).unwrap();
        assert!((bpm - 120.0).abs() < 0.001, "{bpm}");
        // Uneven beats average the way `aubio tempo` averaged them: the mean
        // of the tempi (eight at 120, one at 105), not of the intervals.
        let uneven = "0.0\n0.5\n1.0\n1.5\n2.0\n2.5\n3.0\n3.5\n4.0\n4.571429\n";
        let bpm = parse_tempo(uneven).unwrap();
        assert!((bpm - 118.33).abs() < 0.01, "{bpm}");
    }

    #[test]
    fn tempo_rejects_too_few_beats_and_nonsense() {
        // Nine beats: aubio's "uncertain".
        assert_eq!(parse_tempo(&beats(9, 0.5)), None);
        assert_eq!(parse_tempo(""), None);
        assert_eq!(parse_tempo("usage: aubiotrack\n"), None);
        // Detector noise: a "tempo" of 4 BPM or 900 BPM is not music.
        assert_eq!(parse_tempo(&beats(12, 15.0)), None);
        assert_eq!(parse_tempo(&beats(12, 0.0667)), None);
        // Two beats at one time: an infinite tempo is no tempo.
        assert_eq!(parse_tempo("0\n0\n1\n2\n3\n4\n5\n6\n7\n8\n"), None);
    }

    /// A real click track through ffmpeg and aubiotrack. Skipped where they are
    /// not installed (the image has both).
    #[tokio::test]
    async fn tempo_of_a_click_track() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clicks.mp3");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-v", "error", "-f", "lavfi", "-i",
                "aevalsrc='if(lt(mod(t,0.46875),0.03),sin(2*PI*1000*t),0)+0.2*sin(2*PI*220*t)':s=44100:d=40",
                "-c:a", "libmp3lame",
            ])
            .arg(&file)
            .status();
        let tracker = std::process::Command::new("aubiotrack").arg("-h").output();
        if !made.is_ok_and(|s| s.success()) || tracker.is_err() {
            eprintln!("ffmpeg with LAME or aubiotrack not available; skipped");
            return;
        }
        let bpm = tempo(&file).await.unwrap().expect("a click track has a tempo");
        // A click every 0.46875 s is 128 BPM.
        assert!((bpm - 128.0).abs() < 1.0, "{bpm}");
    }

    #[test]
    fn a_flat_curve_has_no_onsets() {
        assert_eq!(count_onsets(&[1.0, 1.0, 1.0, 1.0]), 0);
        assert_eq!(count_onsets(&[]), 0);
    }
}
