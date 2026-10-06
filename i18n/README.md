# i18n — the one catalog of UI strings

Every user-facing string of every audio2 client lives here, once, in English and
Czech. Clients never hold their own copy of a translation: they get generated
files from this catalog. Today only the web app (`frontend/`) is generated from
it; the Apple apps, tvOS, the Mac and Android still carry their own catalogs and
move here one at a time (see *Other platforms* below).

```
i18n/
  strings/<area>.json     the catalog — the only files anyone edits
  GLOSSARY.md             one Czech word per concept, for every app
  scripts/i18n.mjs        check the catalog; generate platform files
```

```bash
cd i18n && npm install        # once
npm run check                 # validate everything (run before committing)
npm run build:web             # regenerate frontend/src/i18n/generated/
```

`cd frontend && npm run i18n` does the last two in one go.

## A string

```json
{
  "home.quickLinks.title": {
    "en": "Quick links",
    "cs": "Rychlé odkazy",
    "note": "Home widget: three big buttons to the libraries."
  }
}
```

- **Keys are stable dotted names, never the English text.** `area.screen.thing`,
  camelCase segments. The first segment is the file: `home.*` lives in
  `strings/home.json`. Changing the English wording keeps the key, so no
  translation is orphaned; a key is only renamed when its *meaning* changes.
- **`note`** tells a translator where the string appears and anything not obvious
  (a verb or a noun, how long it may be). Optional, but write one whenever a word
  could be read two ways ("Play" the verb vs. a play).
- **ICU MessageFormat** in every locale. Placeholders are named: `{name}`,
  `{count, number}`, `{when, date, medium}`. A literal `{` or `'` is quoted as
  `'{'` / `''`.
- **Plurals are whole messages, never glued words.** Czech needs three forms for
  whole numbers:

  ```json
  "music.songCount": {
    "en": "{count, plural, one {# song} other {# songs}}",
    "cs": "{count, plural, one {# skladba} few {# skladby} other {# skladeb}}"
  }
  ```

  The checker refuses a Czech plural without `one`, `few` and `other`.
- **Markup inside a sentence is a tag**, so the translator can move it:
  `"Already have an account? <link>Sign in</link>"`. The code supplies what
  `<link>` renders.
- **Never build a sentence from pieces.** Word order differs between languages;
  one message with placeholders, always.
- **Same placeholders and tags in every locale** — the checker compares them.

## Wording

Czech follows [GLOSSARY.md](GLOSSARY.md) and, beyond it, the wording the Apple
apps already ship (their `Localizable.xcstrings`). Formal address (vykání)
everywhere. Where the web says the same thing as an app, it uses the app's
words. If a string is new to every app, choose the word the glossary implies and
add the term to the glossary when it is a concept others will need.

## Web (`frontend/`)

`npm run build:web` writes `frontend/src/i18n/generated/`: `en.json`, `cs.json`
and `messages.ts` (every key with the type of its placeholders). The generated
files are committed, so the app builds without this folder's tools, and a wrong
key or a missing placeholder is a TypeScript error.

```tsx
const { t, rich, locale } = useT();
t("home.quickLinks.title");
t("music.songCount", { count: tracks.length });
rich("auth.haveAccount", { link: (c) => <button onClick={…}>{c}</button> });
```

- Translate at render time. Module-level constants hold **keys**, and the
  component calls `t()` — a string computed at import time never changes
  language.
- Outside React (toasts, helpers) call `t()` from `src/i18n` directly; it reads
  the current language.
- Dates, numbers and currencies go through `Intl` with `locale` from `useT()`
  (or `intlLocale()` outside React), never `undefined` or a hard-coded `"en-US"`.
- The language is the user's choice in Settings, defaulting to the browser's
  (Czech for `cs-*`, English otherwise), and sets `<html lang>`.
- Text that comes from the server (API errors, titles, metadata) is shown as it
  arrives; only the app's own words are translated.

## Other platforms

The iOS, tvOS, Mac, Android and Windows apps still carry their own catalogs.
How they move here — per-platform aliases first so no UI code changes,
generators for `.xcstrings`, `strings.xml` and `.resw`, and the order — is in
[CLIENTS.md](CLIENTS.md); each client repo has a `LOCALIZATION.md` with its own
to-do list. Until a client is generated from here, its own catalog stays its
source, and any new wording it shares with other clients goes into both.
