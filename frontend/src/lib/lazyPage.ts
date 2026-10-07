// SPDX-License-Identifier: AGPL-3.0-or-later
import { lazy, type ComponentType } from "react";

const RELOADED = "own-audio-chunk-reload";

/* A page loaded on demand. After a deploy, a tab still running the previous
   build asks for chunk names the server no longer has; reloading once picks up
   the new build instead of leaving a broken screen. The flag stops a loop when
   the chunk is missing for some other reason. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the same bound React's own lazy() uses
export function lazyPage<T extends ComponentType<any>>(load: () => Promise<{ default: T }>) {
  return lazy(async () => {
    try {
      const mod = await load();
      try { sessionStorage.removeItem(RELOADED); } catch { /* storage may be blocked */ }
      return mod;
    } catch (err) {
      let reloaded = false;
      try { reloaded = sessionStorage.getItem(RELOADED) === "1"; sessionStorage.setItem(RELOADED, "1"); } catch { /* storage may be blocked */ }
      if (!reloaded) {
        window.location.reload();
        return new Promise<{ default: T }>(() => {});
      }
      throw err;
    }
  });
}
