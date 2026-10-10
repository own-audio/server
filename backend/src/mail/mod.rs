// SPDX-License-Identifier: AGPL-3.0-or-later
//! Outbound transactional mail over SMTP submission (`MAIL__SMTP_*`).
//!
//! Every call site treats a send failure as best-effort: an invite (or any
//! future transactional mail) must succeed even if the mailer is unset or
//! the mail server is unreachable. Callers log the error and carry on —
//! never fail the caller's request over a mail hiccup.

use crate::app::config::MailConfig;

pub mod invite;
pub mod lockout;
mod smtp;

/// Send a plain-text + HTML email. `config: None`, or one without an SMTP
/// host, is a logged no-op.
pub async fn send_mail(
    config: Option<&MailConfig>,
    to: &str,
    subject: &str,
    text: &str,
    html: &str,
) -> anyhow::Result<()> {
    match config {
        Some(cfg) if cfg.smtp_host().is_some() => smtp::send(cfg, to, subject, text, html).await,
        _ => {
            tracing::warn!(to, "mail not configured — skipping send");
            Ok(())
        }
    }
}
