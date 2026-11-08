// SPDX-License-Identifier: AGPL-3.0-or-later
//! "Set a new password" — the link behind `POST /auth/password/forgot`.
//! Best effort like every mail here.
use super::layout::{button, escape, literal, note, page, paragraph};
use super::Lang;
use crate::app::config::MailConfig;

pub async fn send_password_reset(config: Option<&MailConfig>, lang: Lang, to: &str, reset_url: &str, valid_minutes: i64) {
    let (subject, text, html) = render(lang, to, reset_url, valid_minutes);
    if let Err(err) = super::send_mail(config, to, &subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "password reset mail not sent");
    }
}

fn render(lang: Lang, to: &str, url: &str, minutes: i64) -> (String, String, String) {
    let e = escape(to);
    match lang {
        Lang::En => (
            "Set a new own.audio password".into(),
            format!(
                "Hello,\n\n\
                 someone — hopefully you — asked for a new password for your own.audio account ({to}).\n\n\
                 Open this link to set one:\n{url}\n\n\
                 The link works once and for {minutes} minutes. If you did not ask for it, ignore this mail; \
                 your password stays as it is.\n"
            ),
            page(lang, "Set a new password", &[
                paragraph(&format!("Someone — hopefully you — asked for a new password for your own.audio account (<strong>{e}</strong>).")),
                button(url, "Choose a password"),
                note("If the button doesn't work, open this address:"),
                literal(url, false),
                note(&format!("The link works once and for {minutes} minutes. If you did not ask for it, ignore this mail; your password stays as it is.")),
            ].concat()),
        ),
        Lang::Cs => (
            "Nové heslo k own.audio".into(),
            format!(
                "Dobrý den,\n\n\
                 někdo — snad vy — požádal o nové heslo k vašemu účtu own.audio ({to}).\n\n\
                 Nové heslo nastavíte na tomto odkazu:\n{url}\n\n\
                 Odkaz funguje jednou a {minutes} minut. Pokud jste o něj nežádali, e-mail ignorujte; \
                 vaše heslo zůstává beze změny.\n"
            ),
            page(lang, "Nastavte nové heslo", &[
                paragraph(&format!("Někdo — snad vy — požádal o nové heslo k vašemu účtu own.audio (<strong>{e}</strong>).")),
                button(url, "Nastavit heslo"),
                note("Pokud tlačítko nefunguje, otevřete tuto adresu:"),
                literal(url, false),
                note(&format!("Odkaz funguje jednou a {minutes} minut. Pokud jste o něj nežádali, e-mail ignorujte; vaše heslo zůstává beze změny.")),
            ].concat()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_carry_the_link_and_the_logo() {
        for lang in [Lang::En, Lang::Cs] {
            let (_, text, html) = render(lang, "a@example.com", "https://x.test/reset?token=1&b=2", 30);
            assert!(text.contains("https://x.test/reset?token=1&b=2"));
            assert!(html.contains("https://x.test/reset?token=1&amp;b=2"));
            assert!(html.contains("cid:own-audio-logo"));
            assert!(html.contains(&format!("lang=\"{}\"", lang.code())));
        }
        assert!(render(Lang::Cs, "a@example.com", "u", 30).0.contains("heslo"));
    }
}
