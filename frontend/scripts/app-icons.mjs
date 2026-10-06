// Home-screen icons for the web app, light and dark, into public/.
//
//   npm run icons
//
// The drawing is the own.audio mark in the app-icon construction: the five
// family members on the ring the Book, Podcast and Music icons use (head 10
// from the edge, r 4.4), the purple core, and the two arcs. On a home screen
// the mark stands still, so the arcs are turned to ten o'clock, where they read
// as sound leaving the core rather than a stand beneath it.
//
// Geometry and colours mirror audio2-www/src/lib/mark.ts, which is the source
// of truth for the mark; nothing is shared between the repos by code.

import { writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const PUBLIC = fileURLToPath(new URL("../public/", import.meta.url));

const FAMILY = ["#FABB05", "#29ABE2", "#FF3B30", "#FFCC00", "#34C759"];
const CORE = "#6E44FF";
const ARCS_AT_TEN = 120; // degrees clockwise from the mark's six o'clock

const THEMES = {
  light: { ground: "#FFFFFF", arc: "#B9B9C0", suffix: "" },
  dark: { ground: "#0C0C0E", arc: "#56565E", suffix: "-dark" },
};

const member = (angle, fill) => `
  <g transform="rotate(${angle} 50 50)">
    <circle cx="50" cy="10" r="4.4" fill="${fill}"/>
    <circle cx="41.5" cy="11" r="2.2" fill="${fill}"/>
    <circle cx="58.5" cy="11" r="2.2" fill="${fill}"/>
  </g>`;

/** `scale` shrinks the drawing about the centre: Android's maskable icon must
 *  keep everything inside the middle 80% circle, which launchers never crop. */
function svg({ ground, arc }, scale = 1) {
  const offset = 50 - 50 * scale;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect width="100" height="100" fill="${ground}"/>
  <g transform="translate(${offset} ${offset}) scale(${scale})">
    ${FAMILY.map((fill, i) => member(i * 72, fill)).join("")}
    <circle cx="50" cy="50" r="12" fill="${CORE}"/>
    <g transform="rotate(${ARCS_AT_TEN} 50 50)" fill="none" stroke="${arc}" stroke-linecap="round">
      <path d="M 35 62 A 20 20 0 0 0 65 62" stroke-width="3.5"/>
      <path d="M 28 70 A 30 30 0 0 0 72 70" stroke-width="2.5" opacity="0.6"/>
    </g>
  </g>
</svg>`;
}

// iOS rounds the corners itself; a transparent pixel would turn black.
const OUTPUTS = [
  { name: "apple-touch-icon", size: 180, scale: 1 },
  { name: "icon-192", size: 192, scale: 1 },
  { name: "icon-512", size: 512, scale: 1 },
  { name: "icon-maskable-512", size: 512, scale: 0.84 },
];

/* The browser-tab icon is the compact mark (brand brief: core plus five plain
   dots at ≤32px — shoulders and the outer arc turn to noise there), keeping one
   arc at ten o'clock so it matches the home-screen icon. No ground: the tab
   bar is the ground, and the arc lightens in a dark one. */
const FAVICON = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <style>.arc{stroke:#9A9AA2}@media (prefers-color-scheme:dark){.arc{stroke:#B9B9C0}}</style>
${FAMILY.map((fill, i) => `  <circle cx="${(50 + 34 * Math.sin((i * 72 * Math.PI) / 180)).toFixed(2)}" cy="${(50 - 34 * Math.cos((i * 72 * Math.PI) / 180)).toFixed(2)}" r="9" fill="${fill}"/>`).join("\n")}
  <circle cx="50" cy="50" r="14" fill="${CORE}"/>
  <path class="arc" d="M 35 62 A 20 20 0 0 0 65 62" transform="rotate(${ARCS_AT_TEN} 50 50)" fill="none" stroke-width="5" stroke-linecap="round"/>
</svg>
`;
await writeFile(`${PUBLIC}favicon.svg`, FAVICON);
console.log("favicon.svg");

for (const theme of Object.values(THEMES)) {
  for (const { name, size, scale } of OUTPUTS) {
    const file = `${PUBLIC}${name}${theme.suffix}.png`;
    const png = await sharp(Buffer.from(svg(theme, scale)), { density: 72 * (size / 100) * 2 })
      .resize(size, size)
      .png({ compressionLevel: 9 })
      .toBuffer();
    await writeFile(file, png);
    console.log(`${name}${theme.suffix}.png`);
  }
}
