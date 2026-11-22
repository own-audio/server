// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useI18n } from "../../i18n";
import { MediaCard, MediaRow } from "../../components/library/MediaCard";
import { AiVoiceBadge, MachineTranslationBadge } from "../../components/ui";
import { bookAiVoiceLine, isAiVoiceBook } from "../../lib/machineTranslation";
import type { AudioBook, GenerationJobStatus, RecentPodcastTranslation } from "../../api/types";
import { EpisodeRow, JobRow } from "./MadeHerePage";

useI18n.setState({ locale: "en" });

const wrap = (node: ReactNode) =>
  renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>
  );

const job = (extra: Partial<GenerationJobStatus> = {}): GenerationJobStatus => ({
  id: "j1",
  title: "Krakatit",
  stages: ["extracting", "translating", "narrating", "assembling"],
  stage: "narrating",
  error: null,
  chapters: [],
  download_url: null,
  book_id: null,
  quoted_price_cents: 1275,
  actual_cost_cents: null,
  charged_price_cents: null,
  currency: "USD",
  source_language: "cs",
  target_language: "en",
  blocks_done: 30,
  blocks_total: 120,
  ...extra,
});

const book = { id: "b1", title: "Krakatit", cover_url: null, language: "en", origin: { kind: "machine_translated", source_language: "cs", translator: "google", voice: "Sarah" } } as AudioBook;

describe("the badge", () => {
  it("is plain text a screen reader reads, in both sizes", () => {
    expect(renderToStaticMarkup(<MachineTranslationBadge />)).toContain("Machine translation");
    const compact = renderToStaticMarkup(<MachineTranslationBadge compact />);
    expect(compact).toContain('class="sr-only">Machine translation');
    expect(compact).not.toMatch(/text-(error|warning)/);
  });

  it("sits on a card and a row only when asked", () => {
    expect(renderToStaticMarkup(<MediaCard kind="audiobook" title="A" machineTranslated />)).toContain("Machine translation");
    expect(renderToStaticMarkup(<MediaRow kind="audiobook" title="A" machineTranslated />)).toContain("Machine translation");
    expect(renderToStaticMarkup(<MediaCard kind="audiobook" title="A" />)).not.toContain("Machine translation");
  });

  it("marks an AI narration, and a translation only as a translation", () => {
    expect(renderToStaticMarkup(<MediaCard kind="audiobook" title="A" aiVoice />)).toContain("AI voice");
    expect(renderToStaticMarkup(<MediaRow kind="audiobook" title="A" aiVoice />)).toContain("AI voice");
    const both = renderToStaticMarkup(<MediaCard kind="audiobook" title="A" machineTranslated aiVoice />);
    expect(both).toContain("Machine translation");
    expect(both).not.toContain("AI voice");
    expect(renderToStaticMarkup(<AiVoiceBadge compact />)).toContain('class="sr-only">AI voice');
  });

  it("tells a narrated book from a translated one", () => {
    expect(isAiVoiceBook({ origin: { kind: "narrated", voice: "Tereza" } })).toBe(true);
    expect(isAiVoiceBook({ origin: { kind: "machine_translated", source_language: "cs", translator: "google" } })).toBe(false);
    expect(isAiVoiceBook({ origin: null })).toBe(false);
    expect(bookAiVoiceLine({ origin: { kind: "narrated", voice: "Tereza" } })).toBe("Read by the AI voice Tereza.");
    expect(bookAiVoiceLine({ origin: { kind: "narrated" } })).toBeNull();
  });
});

describe("Made here rows", () => {
  it("shows a running translation with its languages and progress", () => {
    const html = wrap(<JobRow job={job()} />);
    expect(html).toContain("Czech → English");
    expect(html).toContain("Machine translation");
    expect(html).toContain("Narrating · 30/120 blocks");
    expect(html).toContain('aria-valuenow="30"');
  });

  it("shows a finished book with what it cost and Listen now", () => {
    const html = wrap(<JobRow job={job({ stage: "complete", book_id: "b1", charged_price_cents: 980 })} book={book} />);
    expect(html).toContain("Ready");
    expect(html).toContain("$9.80");
    expect(html).toContain("Listen now");
    expect(html).not.toContain("blocks");
  });

  it("offers the cover actions only on a server that reports them", () => {
    expect(wrap(<JobRow job={job({ stage: "complete", book_id: "b1", cover_regenerations_left: 5 })} book={book} />)).toContain("Change the cover");
    expect(wrap(<JobRow job={job({ stage: "complete", book_id: "b1" })} book={book} />)).not.toContain("Change the cover");
  });

  it("shows a failed job with the server's error and what was charged", () => {
    const html = wrap(<JobRow job={job({ stage: "failed", error: "The voice service stopped.", charged_price_cents: 120 })} />);
    expect(html).toContain("Failed — The voice service stopped.");
    expect(html).toContain("You were charged $1.2000 for the part already made");
    const free = wrap(<JobRow job={job({ stage: "failed", charged_price_cents: 0 })} />);
    expect(free).toContain("You have not been charged.");
  });

  it("names a narration in its own language without the badge", () => {
    const html = wrap(<JobRow job={job({ source_language: "en", target_language: "en" })} />);
    expect(html).toContain("English");
    expect(html).not.toContain("Machine translation");
  });

  const tr = (extra: Partial<RecentPodcastTranslation> = {}): RecentPodcastTranslation => ({
    id: "t1",
    episode_id: "e1",
    source_language: "en",
    target_language: "cs",
    voice_profile_id: "cs-CZ-Chirp3-HD-Achernar",
    status: "complete",
    char_count: 1,
    quoted_price_cents: 1,
    charged_price_cents: 1,
    currency: "USD",
    error: null,
    stream_url: "https://x/t1.m4a",
    notice: "Based on the original work, for personal listening.",
    episode_title: "Episode 1",
    feed_id: "f1",
    feed_title: "The Show",
    image_url: null,
    ...extra,
  });

  it("shows a translated episode with the badge, the line, the notice and Play", () => {
    const html = wrap(<EpisodeRow tr={tr({ machine_translated: true, translator: "google" })} />);
    expect(html).toContain("Machine translation");
    expect(html).toContain("Machine-translated by Google from English to Czech, read by Achernar.");
    expect(html).toContain("Based on the original work, for personal listening.");
    expect(html).toContain("Play the Czech translation");
  });

  it("leaves the badge and line off a server that does not flag translations", () => {
    const html = wrap(<EpisodeRow tr={tr()} />);
    expect(html).not.toContain("Machine translation");
    expect(html).not.toContain("Machine-translated");
  });
});
