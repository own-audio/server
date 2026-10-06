// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";

/** Mirrors IdentifyBatchSheet's majority-album rule. Kept in sync by shape:
 *  the real one reads `r.candidate.album`, this reads a plain album string. */
function majorityAlbum(albums: (string | null)[]): string | null {
  const counts = new Map<string, number>();
  for (const a of albums) if (a) counts.set(a, (counts.get(a) ?? 0) + 1);
  let best: string | null = null;
  let n = 0;
  for (const [album, count] of counts) if (count > n) [best, n] = [album, count];
  return n >= 3 && n > albums.length / 2 ? best : null;
}

const outliers = (albums: (string | null)[]) => {
  const m = majorityAlbum(albums);
  return m == null ? [] : albums.filter((a) => a != null && a !== m);
};

describe("majority album", () => {
  it("flags the odd ones out of a real album identify", () => {
    // The case from a Night at the Opera run: nine agree, two don't.
    const albums = [
      "A Night at the Opera", "A Night at the Opera", "A Night at the Opera",
      "A Night at the Opera", "A Night at the Opera", "A Night at the Opera",
      "A Night at the Opera", "A Night at the Opera", "A Night at the Opera",
      "Deluxe Collection",
      "The Cosmos Rocks at Antwerpen + Paris 2008",
    ];
    expect(majorityAlbum(albums)).toBe("A Night at the Opera");
    expect(outliers(albums)).toEqual([
      "Deluxe Collection",
      "The Cosmos Rocks at Antwerpen + Paris 2008",
    ]);
  });

  it("stays silent when there is no majority — nothing is an outlier", () => {
    // A mixed selection is not an album; calling anything odd there would
    // untick correct matches for no reason.
    const albums = ["A", "B", "C", "D"];
    expect(majorityAlbum(albums)).toBeNull();
    expect(outliers(albums)).toEqual([]);
  });

  it("needs more than a coincidental pair", () => {
    expect(majorityAlbum(["A", "A", "B"])).toBeNull();
    expect(majorityAlbum(["A", "A", "A", "B"])).toBe("A");
  });

  it("ignores matches with no album rather than counting them as odd", () => {
    const albums = ["A", "A", "A", "A", null];
    expect(majorityAlbum(albums)).toBe("A");
    expect(outliers(albums)).toEqual([]);
  });
});
