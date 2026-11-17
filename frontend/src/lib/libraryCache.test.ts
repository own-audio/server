// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { QueryClient, QueryObserver } from "@tanstack/react-query";

const saved = { userId: "u1", savedAt: Date.now() - 1000, data: [{ id: "old" }] };
vi.mock("./offline/db", () => ({
  STORES: { meta: "meta" },
  get: vi.fn(async () => saved),
  put: vi.fn(async () => {}),
}));
vi.mock("../store/authStore", () => ({ useAuthStore: { getState: () => ({ user: { id: "u1" } }) } }));

const { initLibraryCache } = await import("./libraryCache");

describe("the library copy on the device", () => {
  it("is shown at once and still fetched, even when it was saved a second ago", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 30_000 } } });
    initLibraryCache(client);
    await vi.waitFor(() => expect(client.getQueryData(["music-tracks"])).toEqual(saved.data));
    const fetched = vi.fn(async () => [{ id: "new" }]);
    const observer = new QueryObserver(client, { queryKey: ["music-tracks"], queryFn: fetched });
    const unsubscribe = observer.subscribe(() => {});
    await vi.waitFor(() => expect(client.getQueryData(["music-tracks"])).toEqual([{ id: "new" }]));
    expect(fetched).toHaveBeenCalledTimes(1);
    unsubscribe();
  });
});
