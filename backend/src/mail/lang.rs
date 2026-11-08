// SPDX-License-Identifier: AGPL-3.0-or-later
//! The language a mail is written in: the `Accept-Language` of the request
//! that caused it. The console sends the language chosen in its settings, the
//! apps send the phone's. Read it with [`Lang::current`] before spawning the
//! send — a task-local does not follow `tokio::spawn`.

use axum::{extract::Request, http::header, middleware::Next, response::Response};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Cs,
}

tokio::task_local! {
    static REQUEST_LANG: Lang;
}

impl Lang {
    /// The best-weighted language we write in, else English.
    pub fn from_accept_language(value: &str) -> Lang {
        let mut best: Option<(f32, Lang)> = None;
        for part in value.split(',') {
            let mut pieces = part.split(';');
            let tag = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
            let q = pieces
                .find_map(|p| p.trim().strip_prefix("q=").and_then(|q| q.trim().parse::<f32>().ok()))
                .unwrap_or(1.0);
            let lang = match tag.split(['-', '_']).next().unwrap_or("") {
                "cs" => Lang::Cs,
                "en" => Lang::En,
                _ => continue,
            };
            if q > 0.0 && best.is_none_or(|(b, _)| q > b) {
                best = Some((q, lang));
            }
        }
        best.map(|(_, l)| l).unwrap_or_default()
    }

    /// The current request's language; English outside a request.
    pub fn current() -> Lang {
        REQUEST_LANG.try_with(|l| *l).unwrap_or_default()
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Cs => "cs",
        }
    }

    pub fn pick<'a>(self, en: &'a str, cs: &'a str) -> &'a str {
        match self {
            Lang::En => en,
            Lang::Cs => cs,
        }
    }
}

pub async fn middleware(request: Request, next: Next) -> Response {
    let lang = request
        .headers()
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .map(Lang::from_accept_language)
        .unwrap_or_default();
    REQUEST_LANG.scope(lang, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use super::Lang;

    #[test]
    fn picks_by_weight() {
        assert_eq!(Lang::from_accept_language("cs"), Lang::Cs);
        assert_eq!(Lang::from_accept_language("cs-CZ,cs;q=0.9,en;q=0.8"), Lang::Cs);
        assert_eq!(Lang::from_accept_language("en-US,en;q=0.9,cs;q=0.8"), Lang::En);
        assert_eq!(Lang::from_accept_language("de-DE,cs;q=0.5,en;q=0.7"), Lang::En);
        assert_eq!(Lang::from_accept_language("de-DE,cs;q=0.5"), Lang::Cs);
    }

    #[test]
    fn unknown_or_missing_is_english() {
        assert_eq!(Lang::from_accept_language(""), Lang::En);
        assert_eq!(Lang::from_accept_language("de,fr;q=0.8"), Lang::En);
        assert_eq!(Lang::from_accept_language("cs;q=0"), Lang::En);
        assert_eq!(Lang::current(), Lang::En);
    }
}
