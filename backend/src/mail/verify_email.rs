// SPDX-License-Identifier: AGPL-3.0-or-later
//! "Confirm your email address" — the link behind a new account whose
//! address nothing has vouched for yet. Best effort like every mail here.
use crate::app::config::MailConfig;

pub async fn send_verification(config: Option<&MailConfig>, to: &str, verify_url: &str, valid_hours: i64) {
    let subject = "Confirm your email address for own.audio";
    let text = format!(
        "Hello,\n\n\
         this address ({to}) was used to create an own.audio account. Open this link to confirm it is yours:\n\
         {verify_url}\n\n\
         The link works once and for {valid_hours} hours. If you did not create the account, ignore this mail.\n"
    );
    let url = escape_html(verify_url);
    let html = format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light" />
    <title>own.audio</title>
  </head>
  <body style="margin:0; padding:0; background:#f4f4f7; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;">
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f4f4f7;">
      <tr>
        <td align="center" style="padding:40px 20px;">
          <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:520px; background:#ffffff; border-radius:16px; border:1px solid #e5e5e7;">
            <tr>
              <td style="padding:36px 40px 32px;">
                <h1 style="margin:0 0 16px; font-size:22px; font-weight:800; letter-spacing:-0.02em; color:#1d1d1f;">Confirm your email address</h1>
                <p style="margin:0 0 16px; font-size:16px; line-height:1.6; color:#1d1d1f;">This address (<strong>{to}</strong>) was used to create an own.audio account. Confirm it is yours:</p>
                <table role="presentation" cellpadding="0" cellspacing="0" style="margin:8px 0 24px;">
                  <tr><td style="background:#6e44ff; border-radius:10px;">
                    <a href="{url}" style="display:inline-block; padding:14px 28px; font-size:16px; font-weight:700; color:#ffffff; text-decoration:none;">Confirm</a>
                  </td></tr>
                </table>
                <p style="margin:0 0 8px; font-size:13px; line-height:1.6; color:#6e6e73;">If the button doesn't work, open this address:</p>
                <p style="margin:0 0 24px; font-size:13px; word-break:break-all; font-family:ui-monospace,SFMono-Regular,Menlo,monospace; color:#1d1d1f;">{url}</p>
                <p style="margin:0; font-size:13px; line-height:1.6; color:#6e6e73;">The link works once and for {valid_hours} hours. If you did not create the account, ignore this mail.</p>
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>"#,
        to = escape_html(to),
    );
    if let Err(err) = super::send_mail(config, to, subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "verification mail not sent");
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
