// SPDX-License-Identifier: AGPL-3.0-or-later
//! "Someone keeps getting your password wrong" — sent once, when an
//! account's sign-in is first locked (security hardening plan §5.1). Best
//! effort like every mail here.
use crate::app::config::MailConfig;

pub async fn send_lockout_notice(config: Option<&MailConfig>, to: &str, failures: i32) {
    let subject = "Sign-in attempts on your own.audio account";
    let text = format!(
        "Hello,\n\n\
         there were {failures} sign-in attempts with a wrong password for your own.audio account ({to}). \
         Signing in with that email is paused for a short while and the pause grows with every further attempt.\n\n\
         If this was you, nothing to do: wait a moment and try again, or ask your family admin for a new password.\n\
         If it was not you, your password was not guessed — but it is a good moment to change it.\n"
    );
    let html = format!(
        "<p>Hello,</p>\
         <p>there were {failures} sign-in attempts with a wrong password for your own.audio account ({to}). \
         Signing in with that email is paused for a short while and the pause grows with every further attempt.</p>\
         <p>If this was you, nothing to do: wait a moment and try again, or ask your family admin for a new password.<br>\
         If it was not you, your password was not guessed — but it is a good moment to change it.</p>",
        to = escape_html(to)
    );
    if let Err(err) = super::send_mail(config, to, subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "lockout notice not sent");
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
