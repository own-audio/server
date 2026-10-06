// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import type { TrashItem } from "../api/trash";
import { canDelete, daysLeft, groupByDeletion } from "./trash";

function item(over: Partial<TrashItem>): TrashItem {
  return {
    kind: "music_track",
    id: "t",
    title: "Song",
    owner: { id: "u", display_name: "U" },
    trashed_by: null,
    trashed_at: "2026-09-25T10:00:00Z",
    purge_at: "2026-10-25T10:00:00Z",
    size_bytes: 0,
    batch: null,
    restore_charge_micro: 0,
    ...over,
  };
}

describe("canDelete", () => {
  it("lets an owner delete, private or shared", () => {
    expect(canDelete({ is_owner: true, visibility: "private" }, false)).toBe(true);
    expect(canDelete({ is_owner: true, visibility: "family" }, false)).toBe(true);
  });

  it("lets a family admin delete someone else's shared item", () => {
    expect(canDelete({ is_owner: false, visibility: "family" }, true)).toBe(true);
  });

  it("never offers a member someone else's item", () => {
    expect(canDelete({ is_owner: false, visibility: "family" }, false)).toBe(false);
  });

  it("does not offer an admin a private item that is not theirs", () => {
    // Private items of others are invisible to admins; the server answers 404.
    expect(canDelete({ is_owner: false, visibility: "private" }, true)).toBe(false);
  });
});

describe("daysLeft", () => {
  it("counts whole days up, never below zero", () => {
    const now = new Date("2026-09-25T10:00:00Z");
    expect(daysLeft("2026-10-25T10:00:00Z", now)).toBe(30);
    expect(daysLeft("2026-09-25T22:00:00Z", now)).toBe(1);
    expect(daysLeft("2026-09-24T10:00:00Z", now)).toBe(0);
  });
});

describe("groupByDeletion", () => {
  it("keeps one gesture together and puts the newest first", () => {
    const groups = groupByDeletion([
      item({ id: "a", batch: "b1", trashed_at: "2026-09-25T10:00:00Z" }),
      item({ id: "b", batch: "b1", trashed_at: "2026-09-25T10:00:01Z" }),
      item({ id: "c", batch: "b2", trashed_at: "2026-09-26T08:00:00Z" }),
    ]);
    expect(groups.map((g) => g.items.map((i) => i.id))).toEqual([["c"], ["a", "b"]]);
    expect(groups[1].trashedAt).toBe("2026-09-25T10:00:01Z");
  });

  it("gives an item without a batch its own group", () => {
    const groups = groupByDeletion([item({ id: "a" }), item({ id: "b" })]);
    expect(groups).toHaveLength(2);
    expect(groups.every((g) => g.batch === null)).toBe(true);
  });
});
