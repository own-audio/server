// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cover Art Archive client, plus the image sniffing every cover provider uses.
//!
//! Cover Art Archive API: https://coverartarchive.org/
//!
//! **This used to be the MusicBrainz API client too.** Identification now goes
//! through the self-hosted mirror (`super::mirror`), so the Lucene query
//! builder, the entity DTOs, and the process-wide 1 req/s semaphore that made
//! a 15-track album take 15 seconds are all gone. Cover art still comes from
//! third parties over HTTP, which is what remains here.

const CAA_BASE: &str = "https://coverartarchive.org";
const USER_AGENT: &str = "audio2/0.1 (own.audio; https://own.audio)";

// ── Cover Art Archive ───────────────────────────────────────────────────

/// Returns the front cover URL from CAA, verified with a HEAD request so a
/// URL this returns is one the caller can actually download — unlike
/// [`MetadataCandidate::cover_art_url`], which is speculative.
pub async fn get_cover_art_url(mb_release_id: &str) -> anyhow::Result<Option<String>> {
    let url = format!("{CAA_BASE}/release/{mb_release_id}/front-500");

    let resp = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(5))
        .build()?
        .head(&url)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() || r.status().is_redirection() => Ok(Some(url)),
        _ => Ok(None),
    }
}

/// Downloads the bytes at a URL already confirmed by [`get_cover_art_url`].
///
/// The status check is load-bearing, not defensive tidiness: Cover Art Archive
/// answers `HEAD` with a `307` to archive.org, and that redirect target can then
/// answer the `GET` with a `500` whose body is an nginx error page — served, to
/// make it worse, with `Content-Type: image/jpeg`. Without this the error page
/// was stored as the album cover (confirmed live: a 170-byte "500 Internal
/// Server Error" HTML page sitting in storage as a track's artwork).
pub async fn download_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

/// The image format of `bytes`, by magic number, or `None` if it isn't an image
/// this code is willing to store.
///
/// Sniffed rather than taken from `Content-Type` or the URL's extension because
/// both have already been observed lying: Cover Art Archive's `/front-500` URLs
/// carry no extension at all (so the old `contains(".png")` test could only ever
/// say "jpg"), and its failure responses claim `image/jpeg` while carrying HTML.
pub fn sniff_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Webp,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_image_format_recognizes_the_formats_caa_actually_serves() {
        assert_eq!(
            sniff_image_format(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            sniff_image_format(b"\x89PNG\r\n\x1a\n\x00\x00"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            sniff_image_format(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some(ImageFormat::Webp)
        );
    }

    /// The exact payload that got stored as a track's album art before this
    /// existed: Cover Art Archive redirected, the redirect target answered 500,
    /// and the error page came back labelled `image/jpeg`.
    #[test]
    fn sniff_image_format_rejects_an_html_error_page() {
        let nginx_500 = b"<html>\r\n<head><title>500 Internal Server Error</title></head>\r\n";
        assert_eq!(sniff_image_format(nginx_500), None);
    }

    #[test]
    fn sniff_image_format_rejects_empty_and_truncated_input() {
        assert_eq!(sniff_image_format(b""), None);
        assert_eq!(sniff_image_format(&[0xFF, 0xD8]), None);
        // "RIFF" alone is a container marker, not necessarily a WebP.
        assert_eq!(sniff_image_format(b"RIFF\x00\x00\x00\x00AVI "), None);
    }

}
