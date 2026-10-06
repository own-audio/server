// SPDX-License-Identifier: AGPL-3.0-or-later
/* The home-screen icon comes in a light and a dark version. Neither platform
   lets a web app switch icons with the phone's appearance later, but both read
   the page's links at the moment the app is added: iOS takes the current
   apple-touch-icon, Chrome the current manifest. So the links follow the
   phone's appearance (not the app's own theme setting — the icon sits among
   the phone's other apps) up to that moment.

   A page that points the links elsewhere — a shared playlist uses its cover —
   is left alone: only our own files are swapped. */

const LINKS = [
  { selector: 'link[rel="apple-touch-icon"]', light: "/apple-touch-icon.png", dark: "/apple-touch-icon-dark.png" },
  { selector: 'link[rel="manifest"]', light: "/manifest.webmanifest", dark: "/manifest-dark.webmanifest" },
];

function apply(dark: boolean) {
  for (const { selector, light, dark: darkHref } of LINKS) {
    const el = document.head.querySelector(selector);
    const href = el?.getAttribute("href");
    if (!el || (href !== light && href !== darkHref)) continue;
    const want = dark ? darkHref : light;
    if (href !== want) el.setAttribute("href", want);
  }
}

export function initHomeScreenIcon() {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  apply(query.matches);
  query.addEventListener("change", (e) => apply(e.matches));
}
