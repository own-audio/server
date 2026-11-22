// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { useI18n } from "../../i18n";
import { TIERS } from "../../lib/voiceTiers.fixture";
import VoiceTierPicker from "./VoiceTierPicker";

useI18n.setState({ locale: "en" });

const render = (language: string, value = "chirp3_hd") =>
  renderToStaticMarkup(<VoiceTierPicker tiers={TIERS} language={language} value={value} onChange={() => {}} />);

const radios = (html: string) => html.match(/role="radio"[^>]*/g) ?? [];

describe("VoiceTierPicker", () => {
  it("shows one card per tier with its price per hour in the language", () => {
    const html = render("cs");
    expect(radios(html)).toHaveLength(3);
    expect(html).toContain("Chirp 3: HD");
    expect(html).toContain("Gemini 3.8 Flash-Lite TTS");
    expect(html).toContain("About $3.80 per hour of audio");
    expect(html).toContain("About $1.90 per hour of audio");
    expect(html).toContain("About $1.20 per hour of audio");
    expect(html).toContain("The price shown is the most you pay.");
  });

  it("marks the chosen tier and badges only the Preview models", () => {
    const html = render("en", "gemini_flash");
    const checked = radios(html).filter((r) => r.includes('aria-checked="true"'));
    expect(checked).toHaveLength(1);
    expect(html.match(/>Preview</g)).toHaveLength(2);
  });

  it("offers a sample per tier, and disables a tier without voices in the language", () => {
    const html = render("sk");
    expect(html.match(/Play a sample of/g)).toHaveLength(1);
    expect(html.match(/Not available in this language/g)).toHaveLength(2);
    expect(radios(html).filter((r) => r.includes('disabled=""'))).toHaveLength(2);
  });
});
