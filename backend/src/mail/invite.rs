// SPDX-License-Identifier: AGPL-3.0-or-later
//! The one transactional email this backend sends today: "you've been
//! invited to a family." Table-based layout with inline styles (Outlook
//! desktop renders via Word's engine and ignores modern CSS), no external
//! image — self-hosted instances have no guaranteed logo asset to link to,
//! unlike the `audio2-www` marketing site's branded shell it's modeled on.

use crate::app::config::MailConfig;
use chrono::{DateTime, Utc};

pub struct InviteEmail<'a> {
    pub family_name: &'a str,
    pub inviter_name: &'a str,
    pub join_url: &'a str,
    /// Shown as a fallback for anyone who can't click the link.
    pub code: &'a str,
    pub expires_at: DateTime<Utc>,
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn expires_human(expires_at: DateTime<Utc>) -> String {
    expires_at.format("%B %-d, %Y").to_string()
}

fn render_text(email: &InviteEmail) -> String {
    format!(
        "{inviter} invited you to join the {family} family on audio2.\n\n\
         Open this link to join:\n{url}\n\n\
         If the link doesn't work, use this code instead: {code}\n\n\
         This invite expires on {expires}.",
        inviter = email.inviter_name,
        family = email.family_name,
        url = email.join_url,
        code = email.code,
        expires = expires_human(email.expires_at),
    )
}

fn render_html(email: &InviteEmail) -> String {
    let inviter = escape_html(email.inviter_name);
    let family = escape_html(email.family_name);
    let url = escape_html(email.join_url);
    let code = escape_html(email.code);
    let expires = expires_human(email.expires_at);

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light" />
    <meta name="supported-color-schemes" content="light" />
    <title>audio2</title>
  </head>
  <body style="margin:0; padding:0; background:#f4f4f7; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;">
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f4f4f7;">
      <tr>
        <td align="center" style="padding:40px 20px;">
          <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:520px; background:#ffffff; border-radius:16px; border:1px solid #e5e5e7;">
            <tr>
              <td style="padding:36px 40px 8px;">
                <h1 style="margin:0 0 16px; font-size:22px; font-weight:800; letter-spacing:-0.02em; color:#1d1d1f;">You're invited to {family}</h1>
                <p style="margin:0 0 16px; font-size:16px; line-height:1.6; color:#1d1d1f;"><strong>{inviter}</strong> invited you to join the <strong>{family}</strong> family on audio2 — a shared home for audiobooks, podcasts, and music.</p>
                <table role="presentation" cellpadding="0" cellspacing="0" style="margin:8px 0 24px;">
                  <tr><td style="background:#6e44ff; border-radius:10px;">
                    <a href="{url}" style="display:inline-block; padding:14px 28px; font-size:16px; font-weight:700; color:#ffffff; text-decoration:none;">Join {family}</a>
                  </td></tr>
                </table>
                <p style="margin:0 0 8px; font-size:13px; line-height:1.6; color:#6e6e73;">If the button doesn't work, use this code instead:</p>
                <p style="margin:0 0 24px; font-size:20px; font-family:ui-monospace,SFMono-Regular,Menlo,monospace; letter-spacing:0.05em; color:#1d1d1f;">{code}</p>
                <p style="margin:0; font-size:13px; line-height:1.6; color:#6e6e73;">This invite expires on {expires}.</p>
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>"#,
        family = family,
        inviter = inviter,
        url = url,
        code = code,
        expires = expires,
    )
}

/// Send the invite email, or silently no-op when mail isn't configured
/// (`config: None`). Never propagates a hard error to the caller — logs and
/// returns `Ok` either way, since a failed send must never fail invite
/// creation (the code/link still works for out-of-band delivery).
pub async fn send_invite_email(config: Option<&MailConfig>, to: &str, email: InviteEmail<'_>) {
    let subject = format!("{} invited you to {}", email.inviter_name, email.family_name);
    let text = render_text(&email);
    let html = render_html(&email);

    if let Err(err) = super::send_mail(config, to, &subject, &text, &html).await {
        tracing::warn!(to, error = %err, "failed to send family invite email");
    }
}
