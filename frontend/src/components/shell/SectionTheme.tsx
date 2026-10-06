// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import { SectionCloudContext, sectionStyle, type SectionCloud } from "../../lib/sectionTheme";

/**
 * Scopes the accent to a section's own cloud colour, so the controls you use
 * inside Audiobooks are gold, Podcasts sky, Music red.
 *
 * It works by redefining `--accent` and `--accent-text` on a wrapper rather
 * than by touching any component: everything inside that already says
 * `bg-accent` / `text-accent` / `ring-accent` follows along, and removing this
 * wrapper puts the whole section back to brand purple. That is deliberate —
 * it is an experiment, and it should cost one line to undo.
 *
 * The cloud values in `tokens.css` are the brand brief's **text-safe** column,
 * dark enough in light mode and bright enough in dark mode to carry
 * `--on-accent` (which already flips with the theme) at AA.
 *
 * ⚠️ This is in tension with the brand brief §4.2 rule 4 — "purple is the
 * accent and stays that way; cloud colours are additions, not replacements".
 * Worth settling before it spreads further than these four sections.
 *
 * Dialogs and menus render in a portal outside this wrapper; they pick the
 * colour up again from context (`useSectionStyle`).
 */
export type { SectionCloud };

export default function SectionTheme({ cloud, children }: { cloud: SectionCloud; children: ReactNode }) {
  return (
    <SectionCloudContext.Provider value={cloud}>
      <div className="contents" style={sectionStyle(cloud)}>
        {children}
      </div>
    </SectionCloudContext.Provider>
  );
}
