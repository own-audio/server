# i18n — the one catalog of UI strings

Every user-facing string of every audio2 client lives here, once, in English,
Czech, Slovak, Hungarian, German, French, Spanish (Spain) and Italian. Clients never hold their own copy of a translation: they get generated
files from this catalog. Today only the web app (`frontend/`) is generated from
it; the Apple apps, tvOS, the Mac and Android still carry their own catalogs and
move here one at a time (see *Other platforms* below).

```
i18n/
  strings/<area>.json     the catalog — the only files anyone edits
  GLOSSARY.md             one word per concept and language, for every app
  scripts/i18n.mjs        check the catalog; generate platform files
```

```bash
cd i18n && npm install        # once
npm run check                 # validate everything (run before committing)
npm run build:web             # regenerate frontend/src/i18n/generated/
npm run review                # how many machine translations are unchecked, per locale
npm run review -- de          # every unchecked German string, with the English
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

  The checker refuses a plural that lacks a category its locale uses for whole
  numbers. The list comes from `Intl.PluralRules` (CLDR): `one few other` for cs
  and sk, `one other` for en, hu and de, and `one many other` for fr, es and it —
  their `many` is round millions, "1 000 000 de morceaux". Czech and Slovak `many`
  is for fractions and is not needed.
- **Markup inside a sentence is a tag**, so the translator can move it:
  `"Already have an account? <link>Sign in</link>"`. The code supplies what
  `<link>` renders.
- **Never build a sentence from pieces.** Word order differs between languages;
  one message with placeholders, always.
- **Same placeholders and tags in every locale** — the checker compares each one
  with English.
- **French and Italian use the typographic apostrophe `’`.** In ICU an ASCII `'`
  before `{` starts a quoted literal and swallows the placeholder (`l'{name}`).

## Locales, and which are checked

`en` and `cs` are required for every string. `sk`, `hu`, `de`, `fr`, `es` and `it`
may lack a key for a while — a new string before its translations land; `check`
counts the gaps and the app shows the English text for them.

**Machine translations carry a marker.** An entry lists the locales whose text a
machine wrote and no native speaker has checked yet:

```json
"common.action.cancel": {
  "en": "Cancel",
  "cs": "Zrušit",
  "de": "Abbrechen",
  …
  "machine": ["sk", "hu", "de", "fr", "es", "it"]
}
```

All six were machine-translated on 2026-11-22 from the English, with the Czech as a
second reference and following [GLOSSARY.md](GLOSSARY.md). A reviewer fixes the
text and **removes their locale from `machine`** (the field goes when it is empty);
`npm run review -- <locale>` lists what is left. A new string translated by machine
gets the marker too. `check` rejects a marker for a locale with no text, or for
English.

## Wording

Every locale follows [GLOSSARY.md](GLOSSARY.md): its word per concept, and its
section "Address and style per language" (Czech, Slovak and Hungarian formal; German,
Spanish and Italian informal; French vous). Czech, beyond the glossary, follows the
wording the Apple apps already ship (their `Localizable.xcstrings`). Where the web says the same thing as an app, it uses the app's
words. If a string is new to every app, choose the word the glossary implies and
add the term to the glossary when it is a concept others will need.

## Web (`frontend/`)

`npm run build:web` writes `frontend/src/i18n/generated/`: one `<locale>.json` per
locale and `messages.ts` (every key with the type of its placeholders). The generated
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
- The language is the user's choice in Settings, defaulting to the first of the
  browser's languages the app has, by language only (`de-AT` → German, `es-MX` →
  the Spain Spanish there is), else English. It sets `<html lang>` and every
  `Intl` formatter.
- English and Czech are in the bundle; the other locales are separate chunks
  fetched when chosen (the app renders once the browser's language has loaded, so
  there is no flash of English). The service worker precaches them for offline.
- A language is added by adding its code to `LOCALES` in `scripts/i18n.mjs`, its
  own name as `common.language.<code>`, its column in GLOSSARY.md and, in the
  console, its Apple sign-in locale (`AppleSignInButton.tsx`).
- Text that comes from the server (API errors, titles, metadata) is shown as it
  arrives; only the app's own words are translated.

## Other platforms

The iOS, tvOS, Mac, Android and Windows apps still carry their own catalogs.
How they move here — per-platform aliases first so no UI code changes,
generators for `.xcstrings`, `strings.xml` and `.resw`, and the order — is in
[CLIENTS.md](CLIENTS.md); each client repo has a `LOCALIZATION.md` with its own
to-do list. Until a client is generated from here, its own catalog stays its
source, and any new wording it shares with other clients goes into both.
