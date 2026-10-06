// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::{Context, bail};
use bytes::Bytes;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::Path;
use tokio::process::Command;
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct YouTubeChannelSnapshot {
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub channel_id: Option<String>,
    pub channel_url: Option<String>,
    pub image_url: Option<String>,
    pub entries: Vec<YouTubeVideoEntry>,
}

#[derive(Debug, Clone)]
pub struct YouTubeVideoEntry {
    pub video_id: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i32>,
    pub image_url: Option<String>,
    pub watch_url: String,
}

pub fn looks_like_channel_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };

    let Some(host) = url.host_str() else {
        return false;
    };

    if !(host.contains("youtube.com") || host.contains("youtu.be")) {
        return false;
    }

    let path = url.path();
    path.starts_with("/@")
        || path.starts_with("/channel/")
        || path.starts_with("/c/")
        || path.starts_with("/user/")
}

pub async fn fetch_channel(channel_url: &str) -> anyhow::Result<YouTubeChannelSnapshot> {
    let output = Command::new("yt-dlp")
        .args([
            "-J",
            "--flat-playlist",
            "--playlist-end",
            "50",
            "--skip-download",
            channel_url,
        ])
        .output()
        .await
        .context("failed to run yt-dlp for channel metadata")?;

    if !output.status.success() {
        bail!(
            "yt-dlp channel fetch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .context("failed to parse yt-dlp channel JSON")?;

    let title = string_at(&value, &["channel", "title", "uploader"])
        .unwrap_or_else(|| "YouTube Channel".to_string());
    let description = string_at(&value, &["description"]);
    let author = string_at(&value, &["uploader", "channel", "channel_follower_count"])
        .or_else(|| string_at(&value, &["uploader", "channel"]));
    let channel_id = string_at(&value, &["channel_id", "id"]);
    let channel_url_out = string_at(&value, &["webpage_url", "original_url"])
        .or_else(|| channel_id.as_ref().map(|id| format!("https://www.youtube.com/channel/{id}")));
    let image_url = pick_thumbnail(&value);

    let mut entries = Vec::new();
    if let Some(items) = value.get("entries").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(video) = parse_entry(item) {
                entries.push(video);
            }
        }
    }

    Ok(YouTubeChannelSnapshot {
        title,
        description,
        author,
        channel_id,
        channel_url: channel_url_out,
        image_url,
        entries,
    })
}

pub async fn download_audio_to_bytes(video_url: &str) -> anyhow::Result<(Bytes, String)> {
    let temp_dir = std::env::temp_dir().join(format!("audio2-yt-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .with_context(|| format!("failed to create temporary yt-dlp directory for {video_url}"))?;

    let template = temp_dir.join("%(id)s.%(ext)s");
    let output = Command::new("yt-dlp")
        .args([
            "-f",
            "bestaudio[ext=m4a]/bestaudio[ext=webm]/bestaudio",
            "--no-playlist",
            "-o",
            template
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid temp path"))?,
            video_url,
        ])
        .output()
        .await
        .with_context(|| format!("failed to run yt-dlp for audio download: {video_url}"))?;

    if !output.status.success() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        bail!(
            "yt-dlp audio download failed for {video_url}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let mut dir = tokio::fs::read_dir(&temp_dir)
        .await
        .with_context(|| format!("failed to read temporary yt-dlp output directory for {video_url}"))?;

    let mut audio_path = None;
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            audio_path = Some(path);
            break;
        }
    }

    let audio_path = audio_path.ok_or_else(|| anyhow::anyhow!("yt-dlp produced no audio file for {video_url}"))?;
    let bytes = tokio::fs::read(&audio_path)
        .await
        .with_context(|| format!("failed to read downloaded audio file for {video_url}"))?;
    let content_type = mime_type_for_path(&audio_path).to_string();

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    Ok((Bytes::from(bytes), content_type))
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or_default() {
        "m4a" | "mp4" => "audio/mp4",
        "webm" => "audio/webm",
        "opus" => "audio/opus",
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

fn parse_entry(value: &Value) -> Option<YouTubeVideoEntry> {
    let video_id = string_at(value, &["id", "url"])?;
    let title = string_at(value, &["title"])?;
    let watch_url = string_at(value, &["webpage_url", "url"])
        .filter(|url| url.starts_with("http"))
        .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={video_id}"));

    Some(YouTubeVideoEntry {
        video_id: video_id.clone(),
        title,
        description: string_at(value, &["description"]),
        published_at: timestamp_at(value, &["timestamp", "release_timestamp"]),
        duration_secs: int_at(value, &["duration"]),
        image_url: pick_thumbnail(value),
        watch_url,
    })
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn int_at(value: &Value, keys: &[&str]) -> Option<i32> {
    for key in keys {
        if let Some(num) = value.get(*key).and_then(|v| v.as_i64()) {
            if let Ok(out) = i32::try_from(num) {
                return Some(out);
            }
        }
    }
    None
}

fn timestamp_at(value: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    for key in keys {
        if let Some(ts) = value.get(*key).and_then(|v| v.as_i64()) {
            if let Some(dt) = Utc.timestamp_opt(ts, 0).single() {
                return Some(dt);
            }
        }
    }
    None
}

fn pick_thumbnail(value: &Value) -> Option<String> {
    if let Some(thumbnails) = value.get("thumbnails").and_then(|v| v.as_array()) {
        let mut best: Option<(i64, String)> = None;
        for thumb in thumbnails {
            if let Some(url) = thumb.get("url").and_then(|v| v.as_str()) {
                let width = thumb.get("width").and_then(|v| v.as_i64()).unwrap_or_default();
                match &best {
                    Some((best_width, _)) if *best_width >= width => {}
                    _ => best = Some((width, url.to_string())),
                }
            }
        }
        if let Some((_, url)) = best {
            return Some(url);
        }
    }

    value
        .get("thumbnail")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}