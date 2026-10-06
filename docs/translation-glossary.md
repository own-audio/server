# Translation glossary — English ⇄ Czech

The one place terminology is decided for every audio2 client. When two apps
disagree on a word, this file wins; change it here first, then in the apps.
Established 2026-09-02 from a full cross-client audit (2 921 translated pairs,
152 conflicting groups — Android repos fixed the same day, remaining iOS/Mac
deltas listed at the bottom).

**How it was audited:** every en→cs pair was harvested from
`audio2-android-{book,podcast}` (strings.xml) and `audio2-mac` +
`audio2-ios-book` (every `Localizable.xcstrings`), normalised (placeholders,
apostrophes, ellipses), grouped by English text, and diffed. Re-run the same
way after any large translation batch.

## Coordination note (2026-09-02)

Two sessions edited Czech at once and briefly *swapped* wording: an iOS-side
session copied Android's old family strings into `audio2-mac` while the
Android side was adopting Mac's. Resolution: Android adopted the wording the
iOS session landed (the old Android text) for the affected family sentences,
and this glossary now decides. If you are the iOS/Mac session: **treat the
table below as the source of truth, not a snapshot of any repo.**

## Canonical terms

| English | Czech | Why / notes |
|---|---|---|
| Narrator | **Vypravěč** | "Interpret" (performer) was simply wrong; fixed on Android 2026-09-02. |
| Narrate / narration (TTS) | **namluvit / namlouvání** | The standard Czech for voicing a book. Never "načíst/načítání", which reads as *loading data*. |
| Cover (book) | **Obálka** | Not "obal" (packaging). |
| Publisher | **Vydavatel** | |
| Identify book (Google Books flow) | **Určit knihu** | |
| Remove download (device copy) | **Odebrat stažené** | "Odebrat" = remove; never "odstranit/smazat", which reads as destroy — the book/episode survives on the server. |
| Downloads (screen: the items) | **Stažené** | |
| Downloads (settings section: the activity) | **Stahování** | Deliberately different word — process vs items. |
| Downloading… | **Stahování…** | Noun form; never 1st person ("Stahuji…"). Applies to all progress labels: Překládání…, Namlouvání…, Dokončování…, Příprava na serveru…. |
| Download failed | **Stahování selhalo** | |
| Sleep timer | **Časovač vypnutí** | Not the calque "časovač spánku". |
| Now playing | **Právě hraje** | Not "Právě se přehrává". |
| Continue listening | **Pokračovat v poslechu** | Already unanimous. |
| Sessions (listening history) | **Poslechy** | Not "Relace". |
| Sign in / Sign out | **Přihlásit se / Odhlásit se** | Reflexive always; screen title may be the noun "Přihlášení". |
| Sign up with Google | **Registrovat se přes Google** | |
| Add to favorites / Remove from favorites | **Přidat do oblíbených / Odebrat z oblíbených** | Symmetric "do/z" pair; not "k oblíbeným". |
| Admin (family role) | **Správce** | Never the anglicism "admin" in Czech text ("Udělat správcem", "Odebrat práva správce"). |
| Block (a family member) | **Zablokovat** | Perfective; button action. |
| Follow (podcast) / Following | **odebírat / Odebíráte** | Podcast subscription is "odběr" (as in "Odhlásit odběr"); never "sledovat". |
| Feed (RSS) | **kanál** | "RSS kanál"; "feed" only inside a URL context. |
| Unsubscribe from X? | **Odhlásit odběr pořadu X?** | Confirm button "Odhlásit odběr" — must not share a word with the dismiss button. |
| Discover / Discovery languages | **Objevovat / Jazyky objevování** | Not "vyhledávání". |
| Refresh | **Obnovit** | Not "Aktualizovat". |
| Offline Mode | **Režim offline** | Not "Offline režim". |
| Skip forward/back (transport) | **Skok vpřed / Skok zpět** | Settings rows may use "Vpřed/Zpět: %ds". |
| Back/Forward %d seconds (a11y) | **Zpět o %d sekund / Vpřed o %d sekund** | |
| No limit | **Bez omezení** | Not "Bez limitu". |
| Keep at most | **Ponechat nejvýše** | |
| Automatic cleanup | **Automatický úklid** | |
| Nothing downloaded yet. | **Zatím nic staženého.** | |
| Pages (book) | **%d stran** | Bookish genitive; not "stránek". |
| DELETE (typed confirmation token) | **DELETE** (untranslated) | It is a token the user must type, not a label. Button labels use "Smazat". |
| Not interested → hides it… and | **Skryje ji v Nových epizodách a %s.** | No "a navíc:". Clauses: "smaže stažený soubor v zařízení", "smaže kopii na serveru", "označí ji jako přehranou"; empty variant "…Nic se nesmaže a nic se neoznačí jako přehrané." |
| Interested again | **Zase mě zajímá** | |
| Follow (queue conflict dialog) | **Pokračovat tam** | Not "Přepnout". |
| Unticked fields… | **Nezaškrtnutá pole zůstanou beze změny — nic se nemaže.** | ("nemazá" was a typo.) |
| Paste the code, or the link/QR… | **Vložte kód nebo odkaz/QR, který vám někdo poslal.** | No comma before "nebo" here. |
| Quotation marks in Czech | **„…“** | Czech lower-upper quotes, not straight `"`. |

## Sentence-level policy

Terminology must match everywhere; full sentences may differ slightly per
platform (screen titles, button grammar — Android infinitive vs a fuller iOS
phrasing) as long as every term in them follows the table. Don't churn
sentences that only differ in phrasing style.

## Known deltas to apply on the iOS/Mac side

Android is aligned as of 2026-09-02. Still open in `audio2-mac` /
`audio2-ios-book` (owner: the iOS-side session):

- FeatureGeneration (mac + iosbook): "Načíst knihu"/"Načítání" → **Namluvit
  knihu / Namlouvání** (see *narrate*).
- mac:FeaturePodcasts: "Sledujete"→"Odebíráte", "Protože sledujete"→"Protože
  odebíráte", "feed"→"kanál" in prose, "Zrušit odběr %@?"→"Odhlásit odběr
  pořadu %@?", "Aktualizovat"→"Obnovit", "a navíc: %@."→"a %@.",
  "Zase mě to zajímá"→"Zase mě zajímá", "označí jako přehrané"→"označí ji
  jako přehranou", "Nic se nemaže a nic se neoznačuje"→"Nic se nesmaže a nic
  se neoznačí", "Příprava na serveru…" stays.
- mac:FeatureSettings: "Jazyky vyhledávání" → **Jazyky objevování**.
- mac:FeatureLibrary/FeaturePlayer: "Přidat k oblíbeným" → **Přidat do
  oblíbených**.
- mac:FeatureFamily: "Blokovat" → **Zablokovat**; invite-link caption must say
  "správcem se tím nikdy nestane" (no "roli admina"); "Vložte kód, nebo…" →
  drop the comma; label hint quotes → „…“.
- iosbook:FeatureDownloads + mac:FeatureDownloads: "Bez limitu" → **Bez
  omezení**; iosbook "Stahování selhalo" already matches (keep).
- iosbook:FeatureHome: "Offline režim" → **Režim offline**.
- iosbook:FeatureLibrary: "Stažené" for the *Downloaded* badge → **Staženo**;
  "%lld stránek" → **%lld stran**; "Přepnout" (queue follow) → **Pokračovat
  tam**.
- mac:Audio2Mac: a11y "Přeskočit vpřed" → **Skok vpřed**.
