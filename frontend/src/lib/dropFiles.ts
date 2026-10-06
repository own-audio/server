// SPDX-License-Identifier: AGPL-3.0-or-later
/**
 * Files from a drag-and-drop, including the contents of dropped folders.
 *
 * `dataTransfer.files` cannot see inside a directory — dropping a folder puts
 * a single unusable entry there — so the entries API is the only way to get
 * at the files. Two details this depends on:
 *
 * - Every entry must be taken from the `DataTransferItemList` **synchronously**,
 *   before the first `await`: the DataTransfer is neutered once the drop
 *   handler yields.
 * - `readEntries` returns at most 100 entries per call and must be called
 *   until it returns an empty batch. Reading it once silently truncates any
 *   folder with more than 100 files in it.
 */
export interface DroppedFile {
  file: File;
  /** Path relative to the drop, e.g. "Album/01 Track.mp3". "" for a loose file. */
  relativePath: string;
}

export async function filesFromDrop(items: DataTransferItemList): Promise<DroppedFile[]> {
  type Entry = { isFile: boolean; isDirectory: boolean; name: string; fullPath?: string };
  type FileEntry = Entry & { file: (cb: (f: File) => void, err?: (e: DOMException) => void) => void };
  type DirEntry = Entry & { createReader: () => { readEntries: (cb: (e: Entry[]) => void, err?: (e: DOMException) => void) => void } };

  const out: DroppedFile[] = [];
  async function walk(entry: Entry, prefix: string) {
    if (entry.isFile) {
      const file = await new Promise<File>((res, rej) => (entry as FileEntry).file(res, rej));
      out.push({ file, relativePath: prefix ? `${prefix}/${entry.name}` : entry.name });
      return;
    }
    const reader = (entry as DirEntry).createReader();
    for (;;) {
      const batch = await new Promise<Entry[]>((res, rej) => reader.readEntries(res, rej));
      if (batch.length === 0) break;
      for (const child of batch) await walk(child, prefix ? `${prefix}/${entry.name}` : entry.name);
    }
  }

  const entries: Entry[] = [];
  for (const item of Array.from(items)) {
    const entry = (item as DataTransferItem & { webkitGetAsEntry?: () => unknown }).webkitGetAsEntry?.();
    if (entry) entries.push(entry as Entry);
  }
  for (const e of entries) await walk(e, "");
  return out;
}

/** Natural order by path, so "10" follows "9" rather than "1". */
export function byPath(a: DroppedFile, b: DroppedFile): number {
  return a.relativePath.localeCompare(b.relativePath, undefined, { numeric: true, sensitivity: "base" });
}
