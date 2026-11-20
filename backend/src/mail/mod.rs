// SPDX-License-Identifier: AGPL-3.0-or-later
//! Outbound transactional mail over SMTP submission (`MAIL__SMTP_*`).
//!
//! Every call site treats a send failure as best-effort: an invite (or any
//! future transactional mail) must succeed even if the mailer is unset or
//! the mail server is unreachable. Callers log the error and carry on —
//! never fail the caller's request over a mail hiccup.

use crate::app::config::MailConfig;

pub mod invite;
pub mod lang;
pub mod layout;
pub mod lockout;
pub mod password_reset;
pub mod verify_email;
pub mod new_device;
mod smtp;

pub use lang::Lang;

/// Send a plain-text + HTML email. `config: None`, or one without an SMTP
/// host, is a logged no-op. An HTML body drawn with [`layout::page`] gets
/// the logo attached inline.
pub async fn send_mail(
    config: Option<&MailConfig>,
    to: &str,
    subject: &str,
    text: &str,
    html: &str,
) -> anyhow::Result<()> {
    if is_reserved(to) {
        // Mail to these never arrives: the provider bounces it back to our own mailbox.
        tracing::debug!(to, "reserved test domain — skipping send");
        return Ok(());
    }
    match config {
        Some(cfg) if cfg.smtp_host().is_some() => smtp::send(cfg, to, subject, text, html).await,
        _ => {
            tracing::warn!(to, "mail not configured — skipping send");
            Ok(())
        }
    }
}

/// Whether the address is on a domain reserved for examples and tests (RFC 2606, RFC 6761):
/// the conformance suite's accounts, for one.
pub fn is_reserved(email: &str) -> bool {
    let Some((_, domain)) = email.trim().rsplit_once('@') else { return false };
    let domain = domain.trim().trim_end_matches(['.', '>']).to_ascii_lowercase();
    let under = |d: &str| domain == d || domain.ends_with(&format!(".{d}"));
    ["example.com", "example.net", "example.org", "example", "test", "invalid", "localhost", "local"]
        .iter()
        .any(|d| under(d))
}

#[cfg(test)]
mod tests {
    use super::is_reserved;

    #[test]
    fn reserved_test_domains() {
        assert!(is_reserved("v2-owner-7bd6505c@example.com"));
        assert!(is_reserved("A@Mail.Example.org"));
        assert!(is_reserved("a@host.test"));
        assert!(is_reserved("a@x.invalid"));
        assert!(!is_reserved("a@example.cz"));
        assert!(!is_reserved("a@notexample.com"));
        assert!(!is_reserved("a@gmail.com"));
        assert!(!is_reserved("no-at-sign"));
    }

    #[tokio::test]
    async fn nothing_goes_to_a_reserved_domain() {
        let cfg: crate::app::config::MailConfig = serde_json::from_value(serde_json::json!({
            "from_address": "a@own.audio", "smtp_host": "127.0.0.1", "smtp_port": 1
        }))
        .unwrap();
        // Port 1 refuses: a send that was attempted would fail.
        assert!(super::send_mail(Some(&cfg), "x@example.com", "s", "t", "h").await.is_ok());
        assert!(super::send_mail(Some(&cfg), "x@own.audio", "s", "t", "h").await.is_err());
    }
}
