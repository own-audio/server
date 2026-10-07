// SPDX-License-Identifier: AGPL-3.0-or-later
//! Podcast search through Apple's public iTunes Search API, for a server
//! without the metadata service's catalogue (`super::mirror`).
//!
//! The search term goes to Apple, which the catalogue avoided; that is why
//! `ITUNES__ENABLED=false` exists and the install guide says so. Only search:
//! categories and similar shows need the catalogue.

use anyhow::{Context, bail};
use serde::Deserialize;
use std::time::Duration;

/// One show as the search answers it.
pub struct Show {
    pub title: String,
    pub feed_url: String,
    pub link: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub episode_count: Option<i32>,
    pub categories: Vec<String>,
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    results: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(default)]
    collection_name: Option<String>,
    #[serde(default)]
    feed_url: Option<String>,
    #[serde(default)]
    collection_view_url: Option<String>,
    #[serde(default)]
    artist_name: Option<String>,
    #[serde(default)]
    artwork_url600: Option<String>,
    #[serde(default)]
    artwork_url100: Option<String>,
    #[serde(default)]
    track_count: Option<i32>,
    #[serde(default)]
    genres: Vec<String>,
}

/// Shows matching `term`, best first as Apple ranks them. Shows without a
/// public feed (Apple-only subscriptions) are left out: there is nothing to
/// subscribe to.
pub async fn search(term: &str, limit: usize) -> anyhow::Result<Vec<Show>> {
    let limit = limit.clamp(1, 50).to_string();
    let response = reqwest::Client::builder()
        .user_agent(concat!("own.audio-server/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()?
        .get("https://itunes.apple.com/search")
        .query(&[("media", "podcast"), ("entity", "podcast"), ("term", term), ("limit", &limit)])
        .send()
        .await
        .context("could not reach itunes.apple.com")?;
    if !response.status().is_success() {
        bail!("itunes.apple.com answered HTTP {}", response.status());
    }
    let answer: Answer = response.json().await.context("unexpected answer from itunes.apple.com")?;
    Ok(shows(answer))
}

fn shows(answer: Answer) -> Vec<Show> {
    answer
        .results
        .into_iter()
        .filter_map(|i| {
            Some(Show {
                title: i.collection_name?,
                feed_url: i.feed_url.filter(|u| !u.is_empty())?,
                link: i.collection_view_url,
                author: i.artist_name,
                image_url: i.artwork_url600.or(i.artwork_url100),
                episode_count: i.track_count.filter(|n| *n > 0),
                // "Podcasts" is on every show; it says nothing.
                categories: i.genres.into_iter().filter(|g| g != "Podcasts").map(|g| g.to_lowercase()).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_shows_and_skips_ones_without_a_feed() {
        let json = r#"{"resultCount":2,"results":[
          {"collectionName":"Hardcore History","feedUrl":"https://feeds.example/hh","artistName":"Dan Carlin",
           "artworkUrl600":"https://img/600.jpg","trackCount":12,"genres":["History","Podcasts"],
           "collectionViewUrl":"https://podcasts.apple.com/x"},
          {"collectionName":"Apple Only","artistName":"Someone"}]}"#;
        let shows = shows(serde_json::from_str(json).unwrap());
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].feed_url, "https://feeds.example/hh");
        assert_eq!(shows[0].categories, vec!["history"]);
        assert_eq!(shows[0].episode_count, Some(12));
    }

    /// `cargo test live_itunes -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_itunes() {
        let found = search("hardcore history", 5).await.unwrap();
        for s in &found {
            println!("{} | {:?} | {} | {:?}", s.title, s.author, s.feed_url, s.categories);
        }
        assert!(!found.is_empty());
    }
}
