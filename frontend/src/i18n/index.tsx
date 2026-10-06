// SPDX-License-Identifier: AGPL-3.0-or-later
import { Fragment, useMemo, type ReactNode } from "react";
import { create } from "zustand";
import { IntlMessageFormat } from "intl-messageformat";
import en from "./generated/en.json";
import cs from "./generated/cs.json";
import type { MessageKey, MessageParams } from "./generated/messages";

/* The app's own words come from i18n/strings/*.json — the one catalog every
   audio2 client shares (i18n/README.md). This file only picks the language
   and formats the messages; never write user-facing text anywhere else. */

export type { MessageKey } from "./generated/messages";
export type Locale = "en" | "cs";
export type LocalePreference = "system" | Locale;

const CATALOGS: Record<Locale, Record<string, string>> = { en, cs };
const STORAGE_KEY = "own-audio-locale";

/** The first of the browser's languages we have, else English. */
function systemLocale(): Locale {
  const langs = typeof navigator === "undefined" ? [] : (navigator.languages ?? [navigator.language]);
  for (const l of langs) {
    const base = l.toLowerCase().split("-")[0];
    if (base === "cs" || base === "en") return base;
  }
  return "en";
}

function readPreference(): LocalePreference {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "en" || v === "cs") return v;
  } catch {
    // storage unavailable — follow the browser
  }
  return "system";
}

const resolve = (p: LocalePreference): Locale => (p === "system" ? systemLocale() : p);

interface I18nState {
  preference: LocalePreference;
  locale: Locale;
  setPreference: (p: LocalePreference) => void;
}

export const useI18n = create<I18nState>()((set) => ({
  preference: readPreference(),
  locale: resolve(readPreference()),
  setPreference: (preference) => {
    try {
      if (preference === "system") localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, preference);
    } catch {
      // ignore — the choice lasts for this visit
    }
    const locale = resolve(preference);
    document.documentElement.lang = locale;
    set({ preference, locale });
  },
}));

/** Call once before the first render. */
export function initI18n() {
  document.documentElement.lang = useI18n.getState().locale;
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
    const text = CATALOGS[locale][key] ?? CATALOGS.en[key] ?? key;
    f = new IntlMessageFormat(text, locale);
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
