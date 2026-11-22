// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";
import { AxiosError, AxiosHeaders } from "axios";

const get = vi.fn();
const post = vi.fn();
vi.mock("./client", () => ({
  default: { get: (...a: unknown[]) => get(...a), post: (...a: unknown[]) => post(...a) },
}));

const store = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
  clear: () => store.clear(),
});

const { createGenerationJob, listGenerationJobs, startedJobIds } = await import("./generation");

afterEach(() => {
  get.mockReset();
  post.mockReset();
  localStorage.clear();
});

const httpError = (status: number) =>
  new AxiosError("x", String(status), undefined, undefined, {
    status,
    statusText: "",
    headers: {},
    config: { headers: new AxiosHeaders() },
    data: {},
  });

describe("the job list", () => {
  it("reads a page and the cursor to the next", async () => {
    get.mockResolvedValueOnce({ data: { items: [{ id: "j2" }, { id: "j1" }], next: "c2" } });
    expect(await listGenerationJobs()).toEqual({ items: [{ id: "j2" }, { id: "j1" }], next: "c2" });
    expect(get).toHaveBeenCalledWith("/audiobook-gen/jobs", { params: { limit: 50 } });
    get.mockResolvedValueOnce({ data: { items: [] } });
    expect(await listGenerationJobs("c2")).toEqual({ items: [], next: null });
    expect(get).toHaveBeenLastCalledWith("/audiobook-gen/jobs", { params: { limit: 50, cursor: "c2" } });
  });

  it("is null on a server without the list, and fails on anything else", async () => {
    get.mockRejectedValueOnce(httpError(404));
    expect(await listGenerationJobs()).toBeNull();
    get.mockRejectedValueOnce(httpError(405));
    expect(await listGenerationJobs()).toBeNull();
    get.mockRejectedValueOnce(httpError(500));
    await expect(listGenerationJobs()).rejects.toBeInstanceOf(AxiosError);
  });

  it("remembers the jobs started in this browser, newest first", async () => {
    const file = new File(["x"], "b.epub");
    const req = { file, title: "B", sourceLanguage: "en", targetLanguage: "en", voiceProfileId: "v", outputMode: "single_m4b" as const, coverMode: "auto" as const, coverShowTitle: false };
    post.mockResolvedValueOnce({ data: { id: "a" } });
    await createGenerationJob(req);
    post.mockResolvedValueOnce({ data: { id: "b" } });
    await createGenerationJob(req);
    expect(startedJobIds()).toEqual(["b", "a"]);
  });
});
