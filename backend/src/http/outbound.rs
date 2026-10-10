// SPDX-License-Identifier: AGPL-3.0-or-later
//! Requests the server makes on a user's behalf: podcast feeds, episode audio,
//! artwork from a feed. The URL comes from someone else, so without a guard a
//! subscription could point the server at its own database, the Docker
//! network, a cloud metadata address or `localhost:8080` and read the answer
//! (OWASP API7, security hardening plan C3).
//!
//! Every fetch here: `http` or `https` only; the host is resolved first and
//! every address checked to be public; the connection is pinned to exactly
//! those addresses, so a name that changes between the check and the connect
//! (DNS rebinding) gains nothing; redirects are followed by hand, each hop
//! checked the same way; the body is read under a size cap and a stall
//! timeout. `SERVER__OUTBOUND__ALLOW_PRIVATE=true` lifts the address check
//! for a server whose feeds genuinely live on its own network.
use anyhow::{bail, Context};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::{redirect, Client, Response, StatusCode, Url};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const USER_AGENT: &str = concat!("audio2/", env!("CARGO_PKG_VERSION"));

/// A URL the guard will not fetch: wrong scheme, no host, or a private
/// address. The caller's mistake, so handlers answer it as a bad request,
/// never as a server error. Found anywhere in an error's chain with
/// [`is_refused`].
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refused(pub String);

pub fn is_refused(error: &anyhow::Error) -> bool {
    error.chain().any(|e| e.downcast_ref::<Refused>().is_some())
}
/// Connect and per-read stall limits; there is no total limit, because an
/// hour-long episode legitimately takes longer than any sensible one.
const STALL: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;

static ALLOW_PRIVATE: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("SERVER__OUTBOUND__ALLOW_PRIVATE").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
});

/// Whether an address is one the public internet routes to. Everything else
/// (loopback, private ranges, link-local including the cloud metadata address,
/// carrier NAT such as Tailscale's 100.64/10, multicast, reserved) is refused.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            // NAT64 (64:ff9b::/96) carries an IPv4 address in the low 32 bits.
            let seg = v6.segments();
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., a, b] = seg;
                return is_public_v4(Ipv4Addr::new((a >> 8) as u8, a as u8, (b >> 8) as u8, b as u8));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link local
                || (seg[0] & 0xff00) == 0xff00 // multicast
                || seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b)) // 100.64/10 carrier NAT, Tailscale
        || a >= 240) // reserved
}

/// Resolves `url`'s host and refuses it unless every address is public.
async fn checked_addrs(url: &Url) -> anyhow::Result<(String, Vec<SocketAddr>)> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Refused("only http and https addresses can be fetched".into()).into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| Refused("the URL has no host".into()))?
        .trim_matches(['[', ']'])
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .with_context(|| format!("could not resolve {host}"))?
            .collect(),
    };
    if addrs.is_empty() {
        bail!("could not resolve {host}");
    }
    if !*ALLOW_PRIVATE {
        if let Some(bad) = addrs.iter().find(|a| !is_public(a.ip())) {
            return Err(Refused(format!("{host} is a private or local address ({}), which the server will not fetch", bad.ip())).into());
        }
    }
    Ok((host, addrs))
}

/// `GET url` with every guard above, following up to five redirects. The
/// response's body has not been read; use [`bytes`] or [`to_temp`].
pub async fn get(url: &str) -> anyhow::Result<Response> {
    let mut url = Url::parse(url.trim()).context("not a valid URL")?;
    for _ in 0..=MAX_REDIRECTS {
        let (host, addrs) = checked_addrs(&url).await?;
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(STALL)
            .read_timeout(STALL)
            .redirect(redirect::Policy::none())
            .resolve_to_addrs(&host, &addrs)
            .build()
            .context("build outbound client")?;
        let response = client.get(url.clone()).send().await.with_context(|| format!("fetch {url}"))?;
        if response.status().is_redirection() {
            let next = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .context("redirect without a Location header")?;
            url = url.join(next).context("redirect to an invalid URL")?;
            continue;
        }
        if response.status() == StatusCode::OK || response.status().is_success() {
            return Ok(response);
        }
        bail!("{url} answered HTTP {}", response.status());
    }
    bail!("too many redirects")
}

/// The whole body, refused past `max_bytes` rather than read into memory.
pub async fn bytes(response: Response, max_bytes: u64) -> anyhow::Result<Bytes> {
    if response.content_length().is_some_and(|l| l > max_bytes) {
        bail!("refused: the body is larger than {max_bytes} bytes");
    }
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read body")?;
        if out.len() as u64 + chunk.len() as u64 > max_bytes {
            bail!("refused: the body is larger than {max_bytes} bytes");
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(out))
}

/// The body streamed to a temporary file, never held in memory, refused past
/// `max_bytes`. For episode audio.
pub async fn to_temp(response: Response, max_bytes: u64) -> anyhow::Result<(tempfile::NamedTempFile, u64)> {
    if response.content_length().is_some_and(|l| l > max_bytes) {
        bail!("refused: the body is larger than {max_bytes} bytes");
    }
    let temp = tokio::task::spawn_blocking(tempfile::NamedTempFile::new).await?.context("temp file")?;
    let mut file = tokio::fs::File::create(temp.path()).await.context("open temp file")?;
    let mut stream = response.bytes_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read body")?;
        total += chunk.len() as u64;
        if total > max_bytes {
            bail!("refused: the body is larger than {max_bytes} bytes");
        }
        file.write_all(&chunk).await.context("write temp file")?;
    }
    file.flush().await.context("flush temp file")?;
    Ok((temp, total))
}

/// The content type a response declares, or `fallback`.
pub fn content_type(response: &Response, fallback: &str) -> String {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_and_local_addresses_are_not_public() {
        for s in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.254", "192.168.88.40", "169.254.169.254",
            "100.64.0.1", "100.101.102.103", "0.0.0.0", "224.0.0.1", "255.255.255.255", "240.0.0.1",
            "::1", "::", "fc00::1", "fd12::1", "fe80::1", "ff02::1", "::ffff:10.0.0.1", "64:ff9b::7f00:1",
        ] {
            assert!(!is_public(ip(s)), "{s} must be refused");
        }
    }

    #[test]
    fn public_addresses_pass() {
        for s in ["1.1.1.1", "8.8.8.8", "172.32.0.1", "100.128.0.1", "2606:4700:4700::1111", "::ffff:1.1.1.1"] {
            assert!(is_public(ip(s)), "{s} must pass");
        }
    }

    #[tokio::test]
    async fn only_http_and_https() {
        let err = get("file:///etc/passwd").await.unwrap_err().to_string();
        assert!(err.contains("only http and https"), "{err}");
        let err = get("ftp://example.com/x").await.unwrap_err();
        assert!(is_refused(&err), "{err}");
    }

    #[tokio::test]
    async fn a_literal_private_address_is_refused_before_any_connection() {
        for url in ["http://127.0.0.1:1/health", "http://169.254.169.254/latest/meta-data/", "http://[::1]:1/"] {
            let err = get(url).await.unwrap_err();
            assert!(is_refused(&err), "{url}: {err}");
            // Through a context layer too, as handlers see it.
            assert!(is_refused(&err.context("fetch feed")));
        }
    }
}
