// SPDX-License-Identifier: AGPL-3.0-or-later
//! AcoustID: which MusicBrainz recordings a fingerprint belongs to — the one
//! way to identify a song whose tags and file name say nothing ("Track 01").
//!
//! Off unless the operator sets their own application key (`ACOUSTID__API_KEY`,
//! from acoustid.org/new-application). The key and the fingerprint go in a POST
//! body, never in a URL that a proxy or a log would keep. AcoustID asks for at
//! most three requests a second; lookups are counted per month, and an optional
//! monthly limit stops them before a plan's quota does.

use crate::app::config::AcoustidConfig;
use serde::Deserialize;
use sqlx::PgPool;
use std::time::{Duration, Instant};

const DEFAULT_BASE: &str = "https://api.acoustid.org";
/// Three requests a second, AcoustID's limit per application.
const SPACING: Duration = Duration::from_millis(340);
/// A match below this is a different recording more often than not.
pub const MIN_SCORE: f32 = 0.5;

static LAST: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);

pub struct AcoustId {
    http: reqwest::Client,
    base: String,
    key: String,
    monthly_limit: Option<i64>,
}

/// One AcoustID track: a score for how well the fingerprint matched it, and the
/// MusicBrainz recordings linked to it (often one, sometimes a few versions).
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub score: f32,
    pub recording_ids: Vec<String>,
}

impl AcoustId {
    pub fn from_config(config: &AcoustidConfig) -> Option<Self> {
        let key = config.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty())?;
        let http = reqwest::Client::builder()
            .user_agent(format!("own.audio-server/{} ( https://github.com/own-audio/server )", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .ok()?;
        Some(Self {
            http,
            base: config.base_url.clone().unwrap_or_else(|| DEFAULT_BASE.into()).trim_end_matches('/').into(),
            key: key.into(),
            monthly_limit: config.monthly_limit,
        })
    }

    /// The best matches for a fingerprint, best first; empty when nothing
    /// matched or this month's limit is reached.
    pub async fn lookup(&self, pool: &PgPool, fingerprint: &str, duration_secs: i32) -> anyhow::Result<Vec<Match>> {
        if !crate::db::identify::count_acoustid_lookup(pool, self.monthly_limit).await? {
            return Ok(Vec::new());
        }
        self.request(fingerprint, duration_secs).await
    }

    async fn request(&self, fingerprint: &str, duration_secs: i32) -> anyhow::Result<Vec<Match>> {
        {
            let mut last = LAST.lock().await;
            if let Some(at) = *last {
                let wait = SPACING.saturating_sub(at.elapsed());
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            *last = Some(Instant::now());
        }
        let duration = duration_secs.to_string();
        let answer: Answer = self
            .http
            .post(format!("{}/v2/lookup", self.base))
            .form(&[("client", self.key.as_str()), ("duration", &duration), ("fingerprint", fingerprint), ("meta", "recordingids")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(matches(answer))
    }
}

#[derive(Deserialize)]
struct Answer {
    status: String,
    #[serde(default)]
    results: Vec<Result>,
}

#[derive(Deserialize)]
struct Result {
    score: f32,
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Deserialize)]
struct Recording {
    id: String,
}

fn matches(answer: Answer) -> Vec<Match> {
    if answer.status != "ok" {
        return Vec::new();
    }
    let mut out: Vec<Match> = answer
        .results
        .into_iter()
        .filter(|r| r.score >= MIN_SCORE && !r.recordings.is_empty())
        .map(|r| Match { score: r.score, recording_ids: r.recordings.into_iter().map(|x| x.id).collect() })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_answer_best_first_and_drops_weak_or_empty_results() {
        let answer: Answer = serde_json::from_str(
            r#"{"status":"ok","results":[
                {"id":"a","score":0.62,"recordings":[{"id":"r2"}]},
                {"id":"b","score":0.97,"recordings":[{"id":"r1"},{"id":"r1b"}]},
                {"id":"c","score":0.99},
                {"id":"d","score":0.3,"recordings":[{"id":"r3"}]}]}"#,
        )
        .unwrap();
        let m = matches(answer);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].recording_ids, vec!["r1", "r1b"]);
        assert_eq!(m[1].recording_ids, vec!["r2"]);
        let error: Answer = serde_json::from_str(r#"{"status":"error","error":{"code":4,"message":"invalid API key"}}"#).unwrap();
        assert!(matches(error).is_empty());
    }

    #[tokio::test]
    async fn the_key_and_the_fingerprint_go_in_the_body_not_the_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let mut got = Vec::new();
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got);
                if let Some(end) = text.find("\r\n\r\n") {
                    let len: usize = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    if got.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let body = r#"{"status":"ok","results":[{"id":"t","score":0.93,"recordings":[{"id":"rec"}]}]}"#;
            let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len());
            sock.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8(got).unwrap()
        });
        let config = AcoustidConfig { api_key: Some("secret-key".into()), base_url: Some(format!("http://{addr}")), monthly_limit: None };
        let found = AcoustId::from_config(&config).unwrap().request("AQADtFKo", 245).await.unwrap();
        assert_eq!(found, vec![Match { score: 0.93, recording_ids: vec!["rec".into()] }]);
        let request = server.await.unwrap();
        let first_line = request.lines().next().unwrap();
        assert_eq!(first_line, "POST /v2/lookup HTTP/1.1");
        assert!(request.contains("client=secret-key") && request.contains("fingerprint=AQADtFKo") && request.contains("duration=245"));
    }

    #[test]
    fn off_without_a_key() {
        assert!(AcoustId::from_config(&AcoustidConfig::default()).is_none());
        let blank = AcoustidConfig { api_key: Some("  ".into()), ..Default::default() };
        assert!(AcoustId::from_config(&blank).is_none());
    }
}
