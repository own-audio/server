// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { byPath, filesFromDrop } from "./dropFiles";

/** A fake of the entries API, batching reads the way a real one does. */
function dir(name: string, children: unknown[], batchSize = 100) {
  return {
    isFile: false,
    isDirectory: true,
    name,
    createReader() {
      let i = 0;
      return {
        readEntries(cb: (e: unknown[]) => void) {
          const batch = children.slice(i, i + batchSize);
          i += batch.length;
          cb(batch);
        },
      };
    },
  };
}

function file(name: string) {
  return {
    isFile: true,
    isDirectory: false,
    name,
    file: (cb: (f: File) => void) => cb(new File(["x"], name)),
  };
}

function itemsOf(...entries: unknown[]): DataTransferItemList {
  return entries.map((e) => ({ webkitGetAsEntry: () => e })) as unknown as DataTransferItemList;
}

describe("filesFromDrop", () => {
  it("returns loose files with no path prefix", async () => {
    const out = await filesFromDrop(itemsOf(file("a.mp3"), file("b.mp3")));
    expect(out.map((f) => f.relativePath)).toEqual(["a.mp3", "b.mp3"]);
  });

  it("walks a folder and records the path", async () => {
    const out = await filesFromDrop(itemsOf(dir("Album", [file("01.mp3"), file("02.mp3")])));
    expect(out.map((f) => f.relativePath)).toEqual(["Album/01.mp3", "Album/02.mp3"]);
  });

  it("descends through nested folders", async () => {
    const tree = dir("Artist", [dir("Album", [file("01.mp3")]), file("cover.jpg")]);
    const out = await filesFromDrop(itemsOf(tree));
    expect(out.map((f) => f.relativePath)).toEqual(["Artist/Album/01.mp3", "Artist/cover.jpg"]);
  });

  it("reads every entry of a folder holding more than one batch", async () => {
    // readEntries yields at most 100 at a time; stopping at the first batch
    // would silently drop 150 of these.
    const many = Array.from({ length: 250 }, (_, i) => file(`t${i + 1}.mp3`));
    const out = await filesFromDrop(itemsOf(dir("Big", many)));
    expect(out).toHaveLength(250);
  });

  it("ignores items that carry no entry", async () => {
    const items = [{ webkitGetAsEntry: () => null }] as unknown as DataTransferItemList;
    expect(await filesFromDrop(items)).toEqual([]);
  });
});

describe("byPath", () => {
  it("orders numbered tracks naturally", async () => {
    const out = await filesFromDrop(itemsOf(dir("A", [file("10.mp3"), file("9.mp3"), file("1.mp3")])));
    expect(out.sort(byPath).map((f) => f.relativePath)).toEqual(["A/1.mp3", "A/9.mp3", "A/10.mp3"]);
  });
});
