// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { singleFlight } from "./singleFlight";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("singleFlight", () => {
  it("runs the work once for concurrent callers and gives them all the result", async () => {
    let calls = 0;
    const d = deferred<string>();
    const guarded = singleFlight(() => {
      calls++;
      return d.promise;
    });

    const waiters = [guarded(), guarded(), guarded()];
    expect(calls).toBe(1);

    d.resolve("token-1");
    expect(await Promise.all(waiters)).toEqual(["token-1", "token-1", "token-1"]);
    expect(calls).toBe(1);
  });

  it("runs again after the first call settles", async () => {
    let calls = 0;
    const guarded = singleFlight(async () => `token-${++calls}`);

    expect(await guarded()).toBe("token-1");
    expect(await guarded()).toBe("token-2");
  });

  it("shares a failure with every waiter, then allows a retry", async () => {
    let calls = 0;
    const d = deferred<string>();
    const guarded = singleFlight(() => {
      calls++;
      return calls === 1 ? d.promise : Promise.resolve("ok");
    });

    const waiters = [guarded().catch((e) => e), guarded().catch((e) => e)];
    d.reject(new Error("boom"));
    const results = await Promise.all(waiters);
    expect(results.map((r) => (r as Error).message)).toEqual(["boom", "boom"]);

    // A failed refresh must not wedge the lock shut.
    expect(await guarded()).toBe("ok");
    expect(calls).toBe(2);
  });
});
