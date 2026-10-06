// SPDX-License-Identifier: AGPL-3.0-or-later
import { createContext, useContext, type CSSProperties } from "react";

export type SectionCloud = "book" | "podcast" | "music";

export const SectionCloudContext = createContext<SectionCloud | null>(null);

/** The accent variables for a section's cloud colour. */
export function sectionStyle(cloud: SectionCloud): CSSProperties {
  return { "--accent": `var(--${cloud})`, "--accent-text": `var(--${cloud})` } as CSSProperties;
}

/**
 * For anything that renders in a portal — dialogs, menus, popovers. A portal
 * leaves the DOM subtree that carries the section's CSS variables, so it fell
 * back to brand purple; React context still reaches it, so it sets them again.
 */
export function useSectionStyle(): CSSProperties | undefined {
  const cloud = useContext(SectionCloudContext);
  return cloud ? sectionStyle(cloud) : undefined;
}

/** What is playing decides the player's colour, not the page it is shown over. */
export function cloudForKind(kind: "audiobook" | "podcast" | "music"): SectionCloud {
  return kind === "audiobook" ? "book" : kind;
}
