import { expect, test } from "vitest";
import { createThumbQueue, thumbKey } from "./thumbnails";

const noDispose = { concurrency: 2, cacheSize: 10, dispose: () => {} };

test("queue limits concurrency and honours cancel", async () => {
  const started: string[] = [];
  const gates = new Map<string, () => void>();
  const q = createThumbQueue(
    (p) => {
      started.push(p);
      return new Promise<string>((r) => gates.set(p, () => r(p)));
    },
    noDispose,
  );
  const res = ["a", "b", "c", "d"].map((p) => q.request(p === "d" ? "t2" : "t1", p, p, 96));
  expect(started).toEqual(["a", "b"]);
  q.cancelOwner("t2");
  gates.get("a")!();
  await res[0];
  expect(started).toEqual(["a", "b", "c"]);
  expect(await res[3]).toBeNull();
});

test("cache hit and eviction", async () => {
  const disposed: string[] = [];
  let calls = 0;
  const q = createThumbQueue(async (p) => `${p}#${++calls}`, { concurrency: 2, cacheSize: 1, dispose: (v) => disposed.push(v) });
  expect(await q.request("t", "k1", "p1", 96)).toBe("p1#1");
  expect(await q.request("t", "k1", "p1", 96)).toBe("p1#1");
  expect(calls).toBe(1);
  expect(await q.request("t", "k2", "p2", 96)).toBe("p2#2");
  expect(disposed).toEqual(["p1#1"]);
});

test("a failed fetch resolves null and is not cached", async () => {
  let calls = 0;
  const q = createThumbQueue(async () => {
    if (++calls === 1) throw new Error("no thumbnail");
    return "ok";
  }, noDispose);
  expect(await q.request("t", "k", "p", 96)).toBeNull();
  expect(await q.request("t", "k", "p", 96)).toBe("ok");
});

test("the same key requested while in flight is fetched once", async () => {
  let calls = 0;
  const q = createThumbQueue(async () => `v${++calls}`, noDispose);
  expect(await Promise.all([q.request("t", "k", "p", 96), q.request("t", "k", "p", 96)])).toEqual(["v1", "v1"]);
  expect(calls).toBe(1);
});

test("release drops a queued request of one owner but not a started one", async () => {
  const started: string[] = [];
  const gates = new Map<string, () => void>();
  const q = createThumbQueue(
    (p) => {
      started.push(p);
      return new Promise<string>((r) => gates.set(p, () => r(p)));
    },
    { ...noDispose, concurrency: 1 },
  );
  const a = q.request("t", "a", "a", 96);
  const b = q.request("t", "b", "b", 96);
  q.release("t", "b");
  q.release("t", "a"); // già partita: arriva comunque
  gates.get("a")!();
  expect(await a).toBe("a");
  expect(await b).toBeNull();
  expect(started).toEqual(["a"]);
});

test("thumbKey joins path, modification time and size", () => {
  expect(thumbKey("C:\\x\\a.jpg", 5, 96)).toBe("C:\\x\\a.jpg|5|96");
});
