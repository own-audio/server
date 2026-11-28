// SPDX-License-Identifier: AGPL-3.0-or-later
import { Fragment, useMemo, type ReactNode } from "react";
import { create } from "zustand";
import { IntlMessageFormat } from "intl-messageformat";
import en from "./generated/en.json";
import cs from "./generated/cs.json";
import { LOCALES, type MessageKey, type MessageParams } from "./generated/messages";

/* The app's own words come from i18n/strings/*.json — the one catalog every
   audio2 client shares (i18n/README.md). This file only picks the language
   and formats the messages; never write user-facing text anywhere else. */

export type { MessageKey } from "./generated/messages";
export type Locale = (typeof LOCALES)[number];
export type LocalePreference = "system" | Locale;

/* English (the fallback) and Czech ship in the bundle; every other language is a
   chunk of its own, fetched when it is chosen, so no one downloads seven catalogs to
   read one. The service worker precaches the chunks, so they also load offline. */
const CATALOGS: Partial<Record<Locale, Record<string, string>>> = { en, cs };
const LOADERS = import.meta.glob<Record<string, string>>(
  ["./generated/*.json", "!./generated/en.json", "!./generated/cs.json"],
  { import: "default" },
);
const STORAGE_KEY = "own-audio-locale";

/** Every language the app has, in the order the picker lists them. */
export const allLocales: readonly Locale[] = LOCALES;

const isLocale = (v: unknown): v is Locale => typeof v === "string" && (LOCALES as readonly string[]).includes(v);

/** Fetches a locale's catalog if it is not loaded yet. */
async function loadCatalog(locale: Locale): Promise<void> {
  if (CATALOGS[locale]) return;
  const load = LOADERS[`./generated/${locale}.json`];
  if (load) CATALOGS[locale] = await load();
}

/** The first of the browser's languages we have, else English. Only the language
 *  counts, not the region: `de-AT`, `fr-CA` and `es-MX` get the one catalog there is. */
function systemLocale(): Locale {
  const langs = typeof navigator === "undefined" ? [] : (navigator.languages ?? [navigator.language]);
  for (const l of langs) {
    const base = l.toLowerCase().split("-")[0];
    if (isLocale(base)) return base;
  }
  return "en";
}

function readPreference(): LocalePreference {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (isLocale(v)) return v;
  } catch {
    // storage unavailable — follow the browser
  }
  return "system";
}

const resolve = (p: LocalePreference): Locale => (p === "system" ? systemLocale() : p);
/** What can be shown before any fetch: the wanted locale if it is bundled, else English. */
const bundled = (l: Locale): Locale => (CATALOGS[l] ? l : "en");

interface I18nState {
  preference: LocalePreference;
  locale: Locale;
  setPreference: (p: LocalePreference) => void;
}

/** Switches to a locale once its catalog is there. A catalog that fails to load
 *  (offline before the first visit cached it) leaves the current language on. */
async function switchTo(locale: Locale): Promise<void> {
  try {
    await loadCatalog(locale);
  } catch {
    return;
  }
  // A later choice may have arrived while this one loaded; only the latest counts.
  if (resolve(useI18n.getState().preference) !== locale) return;
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  useI18n.setState({ locale });
}

export const useI18n = create<I18nState>()((set) => ({
  preference: readPreference(),
  locale: bundled(resolve(readPreference())),
  setPreference: (preference) => {
    try {
      if (preference === "system") localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, preference);
    } catch {
      // ignore — the choice lasts for this visit
    }
    set({ preference });
    void switchTo(resolve(preference));
  },
}));

/** Call once, and render after it settles, so a German browser never sees a flash of English. */
export async function initI18n(): Promise<void> {
  document.documentElement.lang = useI18n.getState().locale;
  await switchTo(resolve(useI18n.getState().preference));
  // "System" follows the browser if its language changes while the app is open.
  window.addEventListener("languagechange", () => {
    const { preference, setPreference } = useI18n.getState();
    if (preference === "system") setPreference("system");
  });
}

/** The locale for `Intl` formatters outside React. */
export const intlLocale = (): Locale => useI18n.getState().locale;

type Args<K extends MessageKey> = MessageParams[K] extends undefined ? [] : [MessageParams[K]];

/** Keys whose message takes no placeholders — the type for a table of labels
 *  kept outside a component and translated when rendered. */
export type PlainKey = { [K in MessageKey]: MessageParams[K] extends undefined ? K : never }[MessageKey];

const cache = new Map<string, IntlMessageFormat>();

function formatter(locale: Locale, key: MessageKey): IntlMessageFormat {
  const id = `${locale}\u0000${key}`;
  let f = cache.get(id);
  if (!f) {
    // A key missing from a locale falls back to English, then to the key itself,
    // so a gap shows as English rather than as nothing.
    // An English fallback is formatted as English, so its plurals follow English rules.
    const own = CATALOGS[locale]?.[key];
    f = own !== undefined ? new IntlMessageFormat(own, locale) : new IntlMessageFormat(CATALOGS.en?.[key] ?? key, "en");
    cache.set(id, f);
  }
  return f;
}

/** A message as plain text, in the current language. */
export function t<K extends MessageKey>(key: K, ...args: Args<K>): string {
  const out = formatter(intlLocale(), key).format(args[0] as Record<string, unknown> | undefined);
  return Array.isArray(out) ? out.join("") : String(out);
}

/** A message with markup: each `<tag>` in it is rendered by the function of the same name. */
export function rich<K extends MessageKey>(key: K, ...args: Args<K>): ReactNode {
  const out = formatter(intlLocale(), key).format<ReactNode>(args[0] as Record<string, ReactNode> | undefined);
  return Array.isArray(out) ? out.map((part, i) => <Fragment key={i}>{part}</Fragment>) : out;
}

/** `t` and `rich` for a component, re-rendering it when the language changes. */
export function useT() {
  const locale = useI18n((s) => s.locale);
  return useMemo(() => ({ t, rich, locale }), [locale]);
}
