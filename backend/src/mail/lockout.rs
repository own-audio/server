// SPDX-License-Identifier: AGPL-3.0-or-later
//! "Someone keeps getting your password wrong" — sent once, when an
//! account's sign-in is first locked (security hardening plan §5.1). Best
//! effort like every mail here.
use super::layout::{escape, page, paragraph};
use super::Lang;
use crate::app::config::MailConfig;

pub async fn send_lockout_notice(config: Option<&MailConfig>, lang: Lang, to: &str, failures: i32) {
    let (subject, text, html) = render(lang, to, failures);
    if let Err(err) = super::send_mail(config, to, &subject, &text, &html).await {
        tracing::warn!(to, error = %format!("{err:#}"), "lockout notice not sent");
    }
}

fn render(lang: Lang, to: &str, failures: i32) -> (String, String, String) {
    let (subject, heading, paragraphs): (&str, &str, [String; 3]) = match lang {
        Lang::En => (
            "Sign-in attempts on your own.audio account",
            "Sign-in paused",
            [
                format!("There were {failures} sign-in attempts with a wrong password for your own.audio account ({to}). Signing in with that email is paused for a short while and the pause grows with every further attempt."),
                "If this was you, nothing to do: wait a moment and try again, or ask your family admin for a new password.".into(),
                "If it was not you, your password was not guessed — but it is a good moment to change it.".into(),
            ],
        ),
        Lang::Cs => (
            "Pokusy o přihlášení k vašemu účtu own.audio",
            "Přihlášení je pozastavené",
            [
                format!("K vašemu účtu own.audio ({to}) proběhlo {failures} pokusů o přihlášení se špatným heslem. Přihlášení s tímto e-mailem je na chvíli pozastavené a s každým dalším pokusem se pauza prodlužuje."),
                "Pokud jste to byli vy, nemusíte nic dělat: chvíli počkejte a zkuste to znovu, nebo požádejte správce rodiny o nové heslo.".into(),
                "Pokud jste to nebyli vy, heslo uhodnuto nebylo — ale je to dobrá chvíle ho změnit.".into(),
            ],
        ),
    };
    let greeting = lang.pick("Hello,", "Dobrý den,");
    let text = format!("{greeting}\n\n{}\n", paragraphs.join("\n\n"));
    let body: String = paragraphs.iter().map(|p| paragraph(&escape(p))).collect();
    (subject.into(), text, page(lang, heading, &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_address() {
        let (_, _, html) = render(Lang::Cs, "a<b>@example.com", 5);
        assert!(html.contains("a&lt;b&gt;@example.com"));
        assert!(html.contains("5 pokusů"));
    }
}
