// SPDX-License-Identifier: AGPL-3.0-or-later
//! "A new device signed in to your account" (security hardening plan §5.2).
//! Sent when a sign-in starts a device chain unlike any the account had in
//! the last 90 days; best effort like every mail here.
use super::layout::{escape, page, paragraph};
use super::Lang;
use crate::app::config::MailConfig;

pub async fn send_new_device_notice(config: Option<&MailConfig>, lang: Lang, to: &str, device: &str, when: &str) {
    let (subject, text, html) = render(lang, to, device, when);
    if let Err(err) = super::send_mail(config, to, &subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "new-device notice not sent");
    }
}

fn render(lang: Lang, to: &str, device: &str, when: &str) -> (String, String, String) {
    let (subject, heading, intro, advice) = match lang {
        Lang::En => (
            "New sign-in to your own.audio account",
            "A new device signed in",
            format!("A new device signed in to your own.audio account ({to}):"),
            "If this was you, there is nothing to do. If not, change your password in the app's settings — that signs every other device out — and check Settings › Devices.",
        ),
        Lang::Cs => (
            "Nové přihlášení k vašemu účtu own.audio",
            "Přihlásilo se nové zařízení",
            format!("K vašemu účtu own.audio ({to}) se přihlásilo nové zařízení:"),
            "Pokud jste to byli vy, nemusíte nic dělat. Pokud ne, změňte si heslo v nastavení aplikace — tím se odhlásí všechna ostatní zařízení — a projděte Nastavení › Zařízení.",
        ),
    };
    let greeting = lang.pick("Hello,", "Dobrý den,");
    let text = format!("{greeting}\n\n{intro}\n\n{device}\n{when}\n\n{advice}\n");
    let body = [
        paragraph(&escape(&intro)),
        paragraph(&format!("<strong>{}</strong><br>{}", escape(device), escape(when))),
        paragraph(&escape(advice)),
    ]
    .concat();
    (subject.into(), text, page(lang, heading, &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_device() {
        for lang in [Lang::En, Lang::Cs] {
            let (_, text, html) = render(lang, "a@example.com", "iPhone <ios>", "2026-11-08 12:00 UTC");
            assert!(text.contains("iPhone <ios>"));
            assert!(html.contains("iPhone &lt;ios&gt;"));
        }
    }
}
