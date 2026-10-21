// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";

const get = vi.fn();
vi.mock("./client", () => ({ default: { get: (...args: unknown[]) => get(...args) } }));

const { getBookProgress } = await import("./playback");

afterEach(() => get.mockReset());

describe("getBookProgress", () => {
  it("takes a started book's place from the list, inside its file", async () => {
    get.mockResolvedValue({
      data: [
        { book_id: "b1", position_secs: 3725, file_id: "f2", file_position_secs: 125, completed: false, updated_at: "2026-10-21T10:00:00Z" },
      ],
    });
    expect(await getBookProgress("b1")).toEqual({
      media_id: "b1",
      position_secs: 125,
      completed: false,
      updated_at: "2026-10-21T10:00:00Z",
      file_id: "f2",
    });
    expect(get).toHaveBeenCalledWith("/playback/books/progress");
  });

  it("is null for a book not started, without asking for that book", async () => {
    get.mockResolvedValue({ data: [] });
    expect(await getBookProgress("b9")).toBeNull();
    expect(get).toHaveBeenCalledTimes(1);
    expect(get).toHaveBeenCalledWith("/playback/books/progress");
  });
});
