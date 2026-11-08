// SPDX-License-Identifier: AGPL-3.0-or-later
//! "You've been invited to a family." Written in the inviter's language: the
//! person invited is usually family, and nobody knows theirs yet.

use super::layout::{button, escape, literal, note, page, paragraph};
use super::Lang;
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

fn expires_human(lang: Lang, expires_at: DateTime<Utc>) -> String {
    match lang {
        Lang::En => expires_at.format("%B %-d, %Y").to_string(),
        Lang::Cs => expires_at.format("%-d. %-m. %Y").to_string(),
    }
}

fn render(lang: Lang, email: &InviteEmail) -> (String, String, String) {
    let (inviter, family, url, code) = (email.inviter_name, email.family_name, email.join_url, email.code);
    let expires = expires_human(lang, email.expires_at);
    let (ei, ef) = (escape(inviter), escape(family));
    match lang {
        Lang::En => (
            format!("{inviter} invited you to {family}"),
            format!(
                "{inviter} invited you to join the {family} family on own.audio.\n\n\
                 Open this link to join:\n{url}\n\n\
                 If the link doesn't work, use this code instead: {code}\n\n\
                 This invite expires on {expires}.\n"
            ),
            page(lang, &format!("You're invited to {family}"), &[
                paragraph(&format!("<strong>{ei}</strong> invited you to join the <strong>{ef}</strong> family on own.audio — a shared home for audiobooks, podcasts, and music.")),
                button(url, &format!("Join {family}")),
                note("If the button doesn't work, use this code instead:"),
                literal(code, true),
                note(&format!("This invite expires on {expires}.")),
            ].concat()),
        ),
        Lang::Cs => (
            format!("{inviter} vás zve do rodiny {family}"),
            format!(
                "{inviter} vás zve do rodiny {family} na own.audio.\n\n\
                 Připojíte se na tomto odkazu:\n{url}\n\n\
                 Pokud odkaz nefunguje, použijte tento kód: {code}\n\n\
                 Pozvánka platí do {expires}.\n"
            ),
            page(lang, &format!("Pozvánka do rodiny {family}"), &[
                paragraph(&format!("<strong>{ei}</strong> vás zve do rodiny <strong>{ef}</strong> na own.audio — společného domova pro audioknihy, podcasty a hudbu.")),
                button(url, &format!("Připojit se k rodině {family}")),
                note("Pokud tlačítko nefunguje, použijte tento kód:"),
                literal(code, true),
                note(&format!("Pozvánka platí do {expires}.")),
            ].concat()),
        ),
    }
}

/// Send the invite email, or silently no-op when mail isn't configured
/// (`config: None`). Never propagates a hard error to the caller — a failed
/// send must never fail invite creation (the code/link still works for
/// out-of-band delivery).
pub async fn send_invite_email(config: Option<&MailConfig>, lang: Lang, to: &str, email: InviteEmail<'_>) {
    let (subject, text, html) = render(lang, &email);
    if let Err(err) = super::send_mail(config, to, &subject, &text, &html).await {
        tracing::warn!(to, error = %err, "failed to send family invite email");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_own_audio_and_escapes_names() {
        let email = InviteEmail {
            family_name: "Smith & Co",
            inviter_name: "<Ann>",
            join_url: "https://x.test/join/ABC",
            code: "ABC",
            expires_at: DateTime::parse_from_rfc3339("2026-11-20T10:00:00Z").unwrap().with_timezone(&Utc),
        };
        let (_, text, html) = render(Lang::En, &email);
        assert!(text.contains("on own.audio") && !text.contains("audio2"));
        assert!(html.contains("Smith &amp; Co") && html.contains("&lt;Ann&gt;"));
        assert!(text.contains("November 20, 2026"));
        let (subject, text, _) = render(Lang::Cs, &email);
        assert!(subject.contains("vás zve"));
        assert!(text.contains("20. 11. 2026"));
    }
}
