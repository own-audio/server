# Moving every client onto the catalog

The web console is generated from `strings/*.json` since 0.1.48. This page is
the plan for the rest of the family: iOS (book, music, podcast), tvOS, the Mac,
Android (book, music, podcast) and Windows. Each client repo has a
`LOCALIZATION.md` with its own state and to-do list; this is the shared part.

## Where each client starts (2026-09-30)

| Client | Today's catalog | Strings | Czech | Keys today |
|---|---|---|---|---|
| Mac (+ the shared Swift packages) | 19 `Localizable.xcstrings` | 1,725 | complete | English text |
| iOS book | 9 `.xcstrings` (app + features) | 346 | complete | English text |
| iOS music | 1 `.xcstrings` | 101 | complete | English text |
| iOS podcast | 2 `.xcstrings` (app + widgets) | 114 | complete | English text |
| tvOS | 4 `.xcstrings` | 179 | complete | English text |
| Android book / music / podcast | `core/strings` `values/` + `values-cs/` | 456 / 566 / 640 | complete | `snake_case` names |
| Windows | `Strings/en-US` + `cs-CZ` `Resources.resw` | 973 | complete | `Area_Name.Property` (x:Uid) |

The translations exist everywhere. What is missing is one source: the same
English sentence is translated separately in up to eight catalogs, and they
drift (the 2026-09-24 audit found ~110 strings to reconcile; Windows still
deviates, see `audio2-win/CATCH_UP_2026-09-24.md` §3).

## The model: aliases first, keys later

Moving a client must not mean rewriting its UI code. So a catalog entry can
carry, per platform, the name that platform uses today:

```json
"library.unknownAuthor": {
  "en": "Unknown author",
  "cs": "Neznámý autor",
  "alias": {
    "apple": "Unknown Author",
    "android": "library_unknown_author",
    "windows": "Library_UnknownAuthor.Text"
  }
}
```

1. **Generate under the alias.** A client's generator writes its native file
   using the alias as the key, so the app's code does not change at all. From
   that commit on, the app's translations come from the catalog.
2. **Move to the shared key when a screen is touched.** New code uses the
   catalog key (`loc("library.unknownAuthor")`, `R.string.library_unknownAuthor`,
   `library.unknownAuthor`); the alias is deleted once nothing references it.
3. **An entry without an alias** is generated under its own key, converted to
   the platform's rules (Android: dots → underscores).

The Apple alias is the English text the Swift code passes to `loc()` /
`Text()` today; for Android and Windows it is the resource name.

## Generators to build (in `scripts/i18n.mjs`)

Each writes the client's native files into a sibling checkout; the generated
files are committed in the client repo, so every client still builds alone.

| Command | Writes | Plurals | Placeholders |
|---|---|---|---|
| `apple <repo> <map>` | one `Localizable.xcstrings` per target | ICU plural → `variations.plural` (one/few/other) with `%lld` | named → positional `%1$@` / `%2$lld` |
| `android <repo>` | `core/strings/src/main/res/values{,-cs}/strings.xml` | `<plurals>` with `one`/`few`/`other` | `%1$s`, `%1$d` |
| `windows <repo>` | `Strings/{en-US,cs-CZ}/Resources.resw` | resw has none — one entry per category (`_one`/`_few`/`_other`) picked in code, or an ICU library for .NET | `{0}` |

- **Which strings go to which target** is a small map per client in
  `i18n/targets/<client>.json` (namespace or key prefix → output file). The
  Apple map matters most: most of the Apple strings live in the Mac repo's
  Swift packages, which the iOS apps consume.
- `check` learns the `alias` field: an alias must be unique within its platform.
- Markup tags (`<b>`, `<link>`) are web-only; an entry a native client uses may
  not contain them.

## Order

1. **Generators and the import tool** in this repo (see `todo/todo.md`).
2. **Android podcast**, then book and music — the most regular format and
   already key-based; proves the alias model.
3. **Mac and the shared Swift packages** — the bulk of the Apple strings, and
   the iOS apps inherit it through the submodule.
4. **iOS book, music, podcast; tvOS** — their app-level catalogs only.
5. **Windows** — last; needs its plural approach decided first.

## Importing a client

For each string in the client's catalog: if the catalog already has the same
meaning (same English in the same context), add the client's alias to that
entry; otherwise add a new entry in the right area, with the client's current
English and Czech. Where two clients translate the same meaning differently,
`GLOSSARY.md` decides; without a glossary entry, a wrong translation loses and
otherwise the more common wording wins — and the glossary gets a row.

## Done means

- The client's native string files are generated, and `git diff` after
  regenerating is empty.
- No hand edits to those files; the client's `CLAUDE.md` says so.
- Czech matches `GLOSSARY.md`; the client's build still passes its own checks
  (Android lint as errors, Xcode "missing localization" warnings, etc.).
- The client's `LOCALIZATION.md` to-do list is ticked off.
