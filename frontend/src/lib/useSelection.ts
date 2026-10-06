// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";

/** Multi-select state for a list or grid: which ids are picked, and the usual
 *  toggle/clear/select-all around them. */
export function useSelection() {
  const [ids, setIds] = useState<Set<string>>(new Set());
  return {
    ids,
    has: (id: string) => ids.has(id),
    toggle: (id: string) =>
      setIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      }),
    clear: () => setIds(new Set()),
    selectAll: (all: string[]) => setIds(new Set(all)),
    count: ids.size,
  };
}
