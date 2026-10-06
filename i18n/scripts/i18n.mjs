// SPDX-License-Identifier: AGPL-3.0-or-later
// The audio2 string catalog: check it, and generate each platform's files.
//
//   node scripts/i18n.mjs check          validate every string in every locale
//   node scripts/i18n.mjs web <outdir>   en.json, cs.json and messages.ts for the web app
//
// Rules are in ../README.md. Everything here reads strings/*.json and nothing
// else: the catalog is the single source, generated files are never edited.

import { readFileSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { join, basename } from "node:path";
import { fileURLToPath } from "node:url";
import { parse, TYPE } from "@formatjs/icu-messageformat-parser";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const STRINGS = join(ROOT, "strings");
export const LOCALES = ["en", "cs"];
const SOURCE = "en";
/** Plural categories a message must cover for whole numbers, per locale (CLDR). */
const PLURALS = { en: ["one", "other"], cs: ["one", "few", "other"] };
const KEY = /^[a-z][a-zA-Z0-9]*(\.[a-zA-Z0-9]+)+$/;

function load() {
  const entries = new Map();
  const errors = [];
  for (const file of readdirSync(STRINGS).filter((f) => f.endsWith(".json")).sort()) {
    const ns = basename(file, ".json");
    let data;
    try {
      data = JSON.parse(readFileSync(join(STRINGS, file), "utf8"));
    } catch (e) {
      errors.push(`${file}: not valid JSON (${e.message})`);
      continue;
    }
    for (const [key, value] of Object.entries(data)) {
      if (!KEY.test(key)) errors.push(`${file}: "${key}" is not a dotted camelCase key`);
      if (!key.startsWith(`${ns}.`)) errors.push(`${file}: "${key}" must start with "${ns}." — a key lives in the file it is named after`);
      entries.set(key, { ...value, file });
    }
  }
  return { entries, errors };
}

/** Argument name → kind, over the whole message including plural and select branches. */
function argsOf(ast, out = new Map(), tags = new Set(), plurals = []) {
  for (const el of ast) {
    switch (el.type) {
      case TYPE.argument:
        if (!out.has(el.value)) out.set(el.value, "text");
        break;
      case TYPE.number:
        out.set(el.value, "number");
        break;
      case TYPE.date:
      case TYPE.time:
        out.set(el.value, "date");
        break;
      case TYPE.select:
        out.set(el.value, "select");
        for (const o of Object.values(el.options)) argsOf(o.value, out, tags, plurals);
        break;
      case TYPE.plural:
        out.set(el.value, "number");
        plurals.push({ name: el.value, ordinal: el.pluralType === "ordinal", options: Object.keys(el.options) });
        for (const o of Object.values(el.options)) argsOf(o.value, out, tags, plurals);
        break;
      case TYPE.tag:
        tags.add(el.value);
        argsOf(el.children, out, tags, plurals);
        break;
      default:
        break;
    }
  }
  return { args: out, tags, plurals };
}

function analyse() {
  const { entries, errors } = load();
  const parsed = new Map();
  for (const [key, entry] of entries) {
    const where = `${entry.file} ${key}`;
    const perLocale = {};
    for (const locale of LOCALES) {
      const text = entry[locale];
      if (typeof text !== "string" || text.trim() === "") {
        errors.push(`${where}: no ${locale} text`);
        continue;
      }
      try {
        perLocale[locale] = argsOf(parse(text));
      } catch (e) {
        errors.push(`${where} [${locale}]: not valid ICU MessageFormat (${e.message}) — a literal { or ' needs quoting as '{' or ''`);
      }
    }
    const src = perLocale[SOURCE];
    for (const locale of LOCALES) {
      const p = perLocale[locale];
      if (!p || !src) continue;
      if (locale !== SOURCE) {
        const a = [...src.args.keys()].sort().join(",");
        const b = [...p.args.keys()].sort().join(",");
        if (a !== b) errors.push(`${where} [${locale}]: placeholders {${b}} differ from ${SOURCE} {${a}}`);
        const ta = [...src.tags].sort().join(",");
        const tb = [...p.tags].sort().join(",");
        if (ta !== tb) errors.push(`${where} [${locale}]: tags <${tb}> differ from ${SOURCE} <${ta}>`);
      }
      for (const pl of p.plurals) {
        if (pl.ordinal) continue;
        const missing = PLURALS[locale].filter((c) => !pl.options.includes(c));
        if (missing.length) errors.push(`${where} [${locale}]: plural {${pl.name}} lacks ${missing.join(", ")}`);
      }
    }
    if (src) parsed.set(key, src);
  }
  return { entries, parsed, errors };
}

function check() {
  const { entries, errors } = analyse();
  if (errors.length) {
    console.error(errors.join("\n"));
    console.error(`\n${errors.length} problem(s) in ${entries.size} strings.`);
    process.exit(1);
  }
  console.log(`${entries.size} strings, ${LOCALES.join(" + ")}: all good.`);
}

const TS_TYPE = { text: "string | number", number: "number", date: "Date | number", select: "string" };

function web(outdir) {
  const { entries, parsed, errors } = analyse();
  if (errors.length) {
    console.error(errors.join("\n"));
    process.exit(1);
  }
  mkdirSync(outdir, { recursive: true });
  const keys = [...entries.keys()].sort();
  for (const locale of LOCALES) {
    const out = Object.fromEntries(keys.map((k) => [k, entries.get(k)[locale]]));
    writeFileSync(join(outdir, `${locale}.json`), JSON.stringify(out, null, 2) + "\n");
  }
  const lines = [
    "// Generated by i18n/scripts/i18n.mjs from i18n/strings/*.json — do not edit.",
    'import type { ReactNode } from "react";',
    "",
    `export const LOCALES = ${JSON.stringify(LOCALES)} as const;`,
    "",
    "/** Placeholders each message takes; `undefined` for none. Tags take a function of their content. */",
    "export interface MessageParams {",
  ];
  for (const k of keys) {
    const { args, tags } = parsed.get(k);
    const fields = [
      ...[...args].map(([name, kind]) => `${JSON.stringify(name)}: ${TS_TYPE[kind]}`),
      ...[...tags].map((name) => `${JSON.stringify(name)}: (chunks: ReactNode[]) => ReactNode`),
    ];
    lines.push(`  ${JSON.stringify(k)}: ${fields.length ? `{ ${fields.join("; ")} }` : "undefined"};`);
  }
  lines.push("}", "", "export type MessageKey = keyof MessageParams;", "");
  writeFileSync(join(outdir, "messages.ts"), lines.join("\n"));
  console.log(`${keys.length} strings → ${outdir}`);
}

const [cmd, arg] = process.argv.slice(2);
if (cmd === "check") check();
else if (cmd === "web" && arg) web(arg);
else {
  console.error("usage: i18n.mjs check | web <outdir>");
  process.exit(2);
}
