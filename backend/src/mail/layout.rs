// SPDX-License-Identifier: AGPL-3.0-or-later
//! The one shell every mail here is drawn in. Table layout with inline styles
//! (Outlook desktop renders with Word's engine and ignores modern CSS). The
//! logo travels inside the message as an inline part (`cid:`), not a link:
//! a self-hosted server may have no public address to load it from.

use super::Lang;

pub const LOGO_CID: &str = "own-audio-logo";
pub static LOGO_PNG: &[u8] = include_bytes!("../../assets/mail/logo.png");

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A paragraph of body text; `html` is already escaped.
pub fn paragraph(html: &str) -> String {
    format!(r#"<p style="margin:0 0 16px; font-size:16px; line-height:1.6; color:#1d1d1f;">{html}</p>"#)
}

/// Small grey text; `html` is already escaped.
pub fn note(html: &str) -> String {
    format!(r#"<p style="margin:0 0 8px; font-size:13px; line-height:1.6; color:#6e6e73;">{html}</p>"#)
}

pub fn button(url: &str, label: &str) -> String {
    format!(
        r#"<table role="presentation" cellpadding="0" cellspacing="0" style="margin:8px 0 24px;">
                  <tr><td style="background:#6e44ff; border-radius:10px;">
                    <a href="{url}" style="display:inline-block; padding:14px 28px; font-size:16px; font-weight:700; color:#ffffff; text-decoration:none;">{label}</a>
                  </td></tr>
                </table>"#,
        url = escape(url),
        label = escape(label),
    )
}

/// A link or code set apart in monospace, for when the button does not work.
pub fn literal(text: &str, large: bool) -> String {
    let size = if large { "font-size:20px; letter-spacing:0.05em;" } else { "font-size:13px; word-break:break-all;" };
    format!(
        r#"<p style="margin:0 0 24px; {size} font-family:ui-monospace,SFMono-Regular,Menlo,monospace; color:#1d1d1f;">{}</p>"#,
        escape(text)
    )
}

/// The whole page: logo, heading, then `body` (built from the parts above).
pub fn page(lang: Lang, heading: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="{code}">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light" />
    <meta name="supported-color-schemes" content="light" />
    <title>own.audio</title>
  </head>
  <body style="margin:0; padding:0; background:#f4f4f7; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;">
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f4f4f7;">
      <tr>
        <td align="center" style="padding:40px 20px;">
          <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:520px; background:#ffffff; border-radius:16px; border:1px solid #e5e5e7;">
            <tr>
              <td style="padding:32px 40px 0;">
                <img src="cid:{cid}" width="155" height="44" alt="own.audio" style="display:block; border:0; outline:none; width:155px; height:44px;" />
              </td>
            </tr>
            <tr>
              <td style="padding:24px 40px 32px;">
                <h1 style="margin:0 0 16px; font-size:22px; font-weight:800; letter-spacing:-0.02em; color:#1d1d1f;">{heading}</h1>
                {body}
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>"#,
        code = lang.code(),
        cid = LOGO_CID,
        heading = escape(heading),
    )
}
