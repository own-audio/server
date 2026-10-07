// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mail over authenticated SMTP submission, the way most providers and
//! self-hosted mail servers accept it.

use crate::app::config::MailConfig;
use anyhow::{Context, bail};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::time::Duration;

#[derive(Debug, PartialEq)]
enum Security {
    Tls,
    StartTls,
    None,
}

fn security(cfg: &MailConfig) -> anyhow::Result<Security> {
    match cfg.smtp_security.as_deref().map(str::trim).unwrap_or("") {
        "" | "tls" => Ok(Security::Tls),
        "starttls" => Ok(Security::StartTls),
        "none" => Ok(Security::None),
        other => bail!("smtp: MAIL__SMTP_SECURITY must be tls, starttls or none, not {other:?}"),
    }
}

fn credentials(cfg: &MailConfig) -> Option<Credentials> {
    let user = cfg.smtp_user.as_deref().filter(|u| !u.is_empty())?;
    Some(Credentials::new(user.to_string(), cfg.smtp_password.clone().unwrap_or_default()))
}

fn transport(cfg: &MailConfig) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
    let host = cfg.smtp_host().context("smtp: no host")?;
    let security = security(cfg)?;
    let creds = credentials(cfg);
    let builder = match security {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
        Security::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
        Security::None => {
            if creds.is_some() {
                bail!("smtp: refusing to send a password without TLS (MAIL__SMTP_SECURITY=none)");
            }
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
        }
    };
    let port = cfg.smtp_port.unwrap_or(match security {
        Security::Tls => 465,
        Security::StartTls => 587,
        Security::None => 25,
    });
    let mut builder = builder.port(port).timeout(Some(Duration::from_secs(20)));
    if let Some(creds) = creds {
        builder = builder.credentials(creds);
    }
    Ok(builder.build())
}

fn message(cfg: &MailConfig, to: &str, subject: &str, text: &str, html: &str) -> anyhow::Result<Message> {
    let from = Mailbox::new(
        Some(cfg.from_name.clone()),
        cfg.from_address.parse().context("smtp: MAIL__FROM_ADDRESS is not an address")?,
    );
    let to: Mailbox = to.parse().context("smtp: recipient is not an address")?;
    Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .multipart(MultiPart::alternative_plain_html(text.to_string(), html.to_string()))
        .context("smtp: building the message")
}

pub(super) async fn send(cfg: &MailConfig, to: &str, subject: &str, text: &str, html: &str) -> anyhow::Result<()> {
    let message = message(cfg, to, subject, text, html)?;
    transport(cfg)?.send(message).await.context("smtp: send failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(security: Option<&str>, user: Option<&str>) -> MailConfig {
        MailConfig {
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: None,
            smtp_user: user.map(Into::into),
            smtp_password: Some("secret".into()),
            smtp_security: security.map(Into::into),
            jmap_base_url: None,
            jmap_user: None,
            jmap_password: None,
            from_address: "hello@example.com".into(),
            from_name: "own.audio".into(),
        }
    }

    #[test]
    fn a_blank_port_from_compose_is_unset() {
        let parse = |v: serde_json::Value| -> MailConfig {
            serde_json::from_value(serde_json::json!({"from_address": "a@example.com", "smtp_port": v})).unwrap()
        };
        assert_eq!(parse(serde_json::json!("")).smtp_port, None);
        assert_eq!(parse(serde_json::json!("587")).smtp_port, Some(587));
        assert_eq!(parse(serde_json::json!(465)).smtp_port, Some(465));
    }

    #[test]
    fn blank_compose_variables_load() {
        #[derive(serde::Deserialize)]
        struct Root {
            mail: Option<MailConfig>,
        }
        let env: std::collections::HashMap<String, String> = [
            ("MAIL__SMTP_HOST", ""), ("MAIL__SMTP_PORT", ""), ("MAIL__SMTP_USER", ""),
            ("MAIL__FROM_ADDRESS", ""), ("MAIL__FROM_NAME", "own.audio"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let cfg = config::Config::builder()
            .add_source(config::Environment::default().separator("__").try_parsing(true).source(Some(env)))
            .build()
            .unwrap();
        let root: Root = cfg.try_deserialize().unwrap();
        let mail = root.mail.unwrap();
        assert_eq!(mail.smtp_port, None);
        assert!(mail.smtp_host().is_none());
    }

    #[test]
    fn security_defaults_to_implicit_tls() {
        assert_eq!(security(&cfg(None, None)).unwrap(), Security::Tls);
        assert_eq!(security(&cfg(Some("starttls"), None)).unwrap(), Security::StartTls);
        assert!(security(&cfg(Some("ssl"), None)).is_err());
    }

    #[test]
    fn no_password_over_plain_text() {
        assert!(transport(&cfg(Some("none"), Some("me"))).is_err());
        assert!(transport(&cfg(Some("none"), None)).is_ok());
    }

    #[test]
    fn header_injection_is_refused() {
        assert!(message(&cfg(None, None), "a@example.com\r\nBcc: b@example.com", "s", "t", "h").is_err());
    }

    #[test]
    fn builds_a_two_part_message() {
        let m = message(&cfg(None, None), "a@example.com", "Hello — ž", "text", "<p>html</p>").unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("From: own.audio <hello@example.com>"));
    }
}
