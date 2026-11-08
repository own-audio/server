// SPDX-License-Identifier: AGPL-3.0-or-later
//! "Confirm your email address" — the link behind a new account whose
//! address nothing has vouched for yet. Best effort like every mail here.
use super::layout::{button, escape, literal, note, page, paragraph};
use super::Lang;
use crate::app::config::MailConfig;

pub async fn send_verification(config: Option<&MailConfig>, lang: Lang, to: &str, verify_url: &str, valid_hours: i64) {
    let (subject, text, html) = render(lang, to, verify_url, valid_hours);
    if let Err(err) = super::send_mail(config, to, &subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "verification mail not sent");
    }
}

fn render(lang: Lang, to: &str, url: &str, hours: i64) -> (String, String, String) {
    let e = escape(to);
    match lang {
        Lang::En => (
            "Confirm your email address for own.audio".into(),
            format!(
                "Hello,\n\n\
                 this address ({to}) was used to create an own.audio account. Open this link to confirm it is yours:\n\
                 {url}\n\n\
                 The link works once and for {hours} hours. If you did not create the account, ignore this mail.\n"
            ),
            page(lang, "Confirm your email address", &[
                paragraph(&format!("This address (<strong>{e}</strong>) was used to create an own.audio account. Confirm it is yours:")),
                button(url, "Confirm"),
                note("If the button doesn't work, open this address:"),
                literal(url, false),
                note(&format!("The link works once and for {hours} hours. If you did not create the account, ignore this mail.")),
            ].concat()),
        ),
        Lang::Cs => (
            "Potvrďte svou e-mailovou adresu pro own.audio".into(),
            format!(
                "Dobrý den,\n\n\
                 s touto adresou ({to}) byl založen účet own.audio. Že je vaše, potvrdíte na tomto odkazu:\n\
                 {url}\n\n\
                 Odkaz funguje jednou a {hours} hodin. Pokud jste účet nezakládali, e-mail ignorujte.\n"
            ),
            page(lang, "Potvrzení e-mailové adresy", &[
                paragraph(&format!("S touto adresou (<strong>{e}</strong>) byl založen účet own.audio. Potvrďte, že je vaše:")),
                button(url, "Potvrdit"),
                note("Pokud tlačítko nefunguje, otevřete tuto adresu:"),
                literal(url, false),
                note(&format!("Odkaz funguje jednou a {hours} hodin. Pokud jste účet nezakládali, e-mail ignorujte.")),
            ].concat()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_carry_the_link() {
        for lang in [Lang::En, Lang::Cs] {
            let (_, text, html) = render(lang, "a@example.com", "https://x.test/verify?token=1", 48);
            assert!(text.contains("https://x.test/verify?token=1"));
            assert!(html.contains("https://x.test/verify?token=1"));
            assert!(html.contains("cid:own-audio-logo"));
        }
    }
}
