// SPDX-License-Identifier: AGPL-3.0-or-later
//! "A new device signed in to your account" (security hardening plan §5.2).
//! Sent when a sign-in starts a device chain unlike any the account had in
//! the last 90 days; best effort like every mail here.
use crate::app::config::MailConfig;

pub async fn send_new_device_notice(config: Option<&MailConfig>, to: &str, device: &str, when: &str) {
    let subject = "New sign-in to your own.audio account";
    let text = format!(
        "Hello,\n\n\
         a new device signed in to your own.audio account ({to}):\n\n\
         {device}\n{when}\n\n\
         If this was you, there is nothing to do. If not, change your password in the app's settings — \
         that signs every other device out — and check Settings › Devices.\n"
    );
    let html = format!(
        "<p>Hello,</p>\
         <p>a new device signed in to your own.audio account ({to}):</p>\
         <p><strong>{device}</strong><br>{when}</p>\
         <p>If this was you, there is nothing to do. If not, change your password in the app's settings — \
         that signs every other device out — and check Settings › Devices.</p>",
        to = escape_html(to),
        device = escape_html(device),
        when = escape_html(when),
    );
    if let Err(err) = super::send_mail(config, to, subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "new-device notice not sent");
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
