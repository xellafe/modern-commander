import { expect, test } from "vitest";
import { lineCache } from "../state/lineCache";
import {
  isLineStart, lastPageTop, lineAt, mergeMarks, nearestMark, nextStart, prevLineStart, queueNav, renderSlice, stepDown,
  stepUp, visibleLines, wholeLines, type NavCtx, type Scans,
} from "./longLines";
import type { Win } from "./textPaging";

const enc = (s: string) => new TextEncoder().encode(s);
const win = (s: string, eof = true, base = 0): Win => ({ base, buf: enc(s), eof });
const utf16le = (s: string) => {
  const out = new Uint8Array(s.length * 2);
  [...s].forEach((c, i) => { out[i * 2] = c.charCodeAt(0); });
  return out;
};

test("wholeLines: lines longer than MAX_LINE stay whole", () => {
  expect(wholeLines(win("x".repeat(10000) + "\nb\n"), 0, 2, 1, false)).toEqual([
    { start: 0, end: 10000, next: 10001 },
    { start: 10001, end: 10002, next: 10003 },
  ]);
});

test("wholeLines: a line whose LF is beyond the window has unknown end", () => {
  expect(wholeLines(win("ab\ncdef", false), 0, 10, 1, false)).toEqual([
    { start: 0, end: 2, next: 3 },
    { start: 3, end: null, next: null },
  ]);
});

test("wholeLines: CRLF end excludes the CR", () => {
  expect(wholeLines(win("ab\r\ncd"), 0, 1, 1, false)).toEqual([{ start: 0, end: 2, next: 4 }]);
  const be = new Uint8Array([0, 0x61, 0, 0x0d, 0, 0x0a, 0, 0x62]);
  expect(wholeLines({ base: 0, buf: be, eof: true }, 0, 1, 2, true)).toEqual([{ start: 0, end: 2, next: 6 }]);
});

test("wholeLines: end of file cases", () => {
  expect(wholeLines(win(""), 0, 10, 1, false)).toEqual([{ start: 0, end: 0, next: 0 }]);
  expect(wholeLines(win("a\nb"), 0, 10, 1, false)).toEqual([
    { start: 0, end: 1, next: 2 },
    { start: 2, end: 3, next: 3 },
  ]);
  expect(wholeLines(win("a\n"), 0, 10, 1, false)).toEqual([
    { start: 0, end: 1, next: 2 },
    { start: 2, end: 2, next: 2 },
  ]);
  expect(wholeLines(win("a\n"), 5, 10, 1, false)).toEqual([]);
});

test("wholeLines: utf-16le with BOM", () => {
  const buf = new Uint8Array([0xff, 0xfe, ...utf16le("a\nb")]);
  const lines = wholeLines({ base: 0, buf, eof: true }, 2, 10, 2, false);
  expect(lines.map((l) => l.start)).toEqual([2, 6]);
  expect(lines[0]).toEqual({ start: 2, end: 4, next: 6 });
});

test("prevLineStart and isLineStart inside and at the edge of the window", () => {
  const w = win("ab\ncd\nef");
  expect(prevLineStart(w, 6, 1, false)).toBe(3);
  expect(prevLineStart(w, 3, 1, false)).toBe(0);
  expect(prevLineStart(w, 0, 1, false)).toBe(0);
  // finestra che parte a 100 senza LF prima di `top`: ignoto
  expect(prevLineStart(win("abcdef", true, 100), 104, 1, false)).toBeNull();
  expect(prevLineStart(win("ab\ncd", true, 100), 105, 1, false)).toBe(103);
  expect(isLineStart(w, 0, 1, false)).toBe(true);
  expect(isLineStart(w, 3, 1, false)).toBe(true);
  expect(isLineStart(w, 4, 1, false)).toBe(false);
  expect(isLineStart(win("abc", true, 100), 100, 1, false)).toBeNull();
  expect(isLineStart(win("a\nb", true, 100), 102, 1, false)).toBe(true);
  expect(isLineStart({ base: 0, buf: new Uint8Array([0x61, 0, 0x0a, 0]), eof: true }, 4, 2, false)).toBe(true);
});

test("nearestMark and mergeMarks", () => {
  const marks = [{ offset: 100, column: 10 }, { offset: 250, column: 30 }, { offset: 400, column: 60 }];
  expect(nearestMark(undefined, 7, 50)).toEqual({ offset: 7, column: 0 });
  expect(nearestMark(marks, 7, 5)).toEqual({ offset: 7, column: 0 });
  expect(nearestMark(marks, 7, 30)).toEqual({ offset: 250, column: 30 });
  expect(nearestMark(marks, 7, 59)).toEqual({ offset: 250, column: 30 });
  expect(nearestMark(marks, 7, 1000)).toEqual({ offset: 400, column: 60 });
  expect(mergeMarks(undefined, marks)).toEqual(marks);
  expect(mergeMarks(marks, [{ offset: 250, column: 30 }, { offset: 50, column: 5 }])).toEqual([
    { offset: 50, column: 5 }, ...marks,
  ]);
});

test("renderSlice: tabs expand on absolute columns", () => {
  expect(renderSlice("a\tb", 0, 0, 20)).toBe("a       b");
  expect(renderSlice("a\tb", 4, 4, 20)).toBe("a   b");
});

test("renderSlice: left inside a tab pads with spaces", () => {
  expect(renderSlice("\tb", 1, 3, 20)).toBe("     b");
});

test("renderSlice: width cuts by columns, a surrogate pair is one", () => {
  expect(renderSlice("abcdef", 0, 0, 3)).toBe("abc");
  expect(renderSlice("a😀b😀c", 0, 0, 4)).toBe("a😀b😀");
  expect(renderSlice("a😀b😀c", 0, 2, 2)).toBe("b😀");
  expect(renderSlice("a\tb", 0, 0, 4)).toBe("a   ");
});

test("queueNav: same direction accumulates, opposite cancels", () => {
  expect(queueNav(null, 1, 3)).toEqual({ pending: { dir: 1, n: 3 }, cancel: false });
  expect(queueNav({ dir: 1, n: 3 }, 1, 2)).toEqual({ pending: { dir: 1, n: 5 }, cancel: false });
  expect(queueNav({ dir: 1, n: 3 }, -1, 2)).toEqual({ pending: { dir: -1, n: 2 }, cancel: true });
});

// Navigazione con un backend simulato sul contenuto intero: registra le chiamate.
const CANCELLED = { code: "CANCELLED", path: null, detail: null };

function backend(data: Uint8Array) {
  const calls: [string, number][] = [];
  const scans: Scans = {
    next: async (from) => {
      calls.push(["next", from]);
      for (let p = from; p < data.length; p++) if (data[p] === 0x0a) return p + 1;
      return data.length;
    },
    start: async (at) => {
      calls.push(["start", at]);
      for (let p = at - 1; p >= 0; p--) if (data[p] === 0x0a) return p + 1;
      return 0;
    },
  };
  return { calls, scans };
}

let tabs = 0;
const ctx = (data: Uint8Array, win: Win | null, scans: Scans, bom = 0): NavCtx => ({
  win, cache: lineCache(`nav${++tabs}`, "utf-8"), unit: 1, bigEndian: false, size: data.length, bom, scans, inflight: new Map(),
});
// Finestra sui byte [base, base + len) di `data`.
const slice = (data: Uint8Array, base: number, len: number): Win => ({
  base, buf: data.subarray(base, base + len), eof: base + len >= data.length,
});
const long = enc("a\n" + "x".repeat(100) + "\nb\nc\n"); // la riga lunga va da 2 a 103

test("stepDown: a line whose LF is beyond the window asks the backend once, then uses the cache", async () => {
  const { calls, scans } = backend(long);
  const c = ctx(long, slice(long, 0, 10), scans);
  expect(await stepDown(c, 0, 1)).toBe(2);
  expect(calls).toEqual([]);
  expect(await stepDown(c, 2, 1)).toBe(103);
  expect(calls).toEqual([["next", 10]]); // riparte dalla fine della finestra
  expect(c.cache.next.get(2)).toBe(103);
  expect(await stepDown(c, 2, 1)).toBe(103);
  expect(calls).toHaveLength(1);
});

test("stepUp: a line start not in the window asks the backend from back.get(top)", async () => {
  const { calls, scans } = backend(long);
  const c = ctx(long, slice(long, 50, 60), scans);
  c.cache.back.set(103, 60);
  expect(await stepUp(c, 103)).toBe(2);
  expect(calls).toEqual([["start", 60]]);
  expect(c.cache.next.get(2)).toBe(103);
  // la seconda volta basta la cache
  expect(await stepUp(c, 103)).toBe(2);
  expect(calls).toHaveLength(1);
  // senza `back` si parte dall'LF sopra `top`
  expect(await stepUp(ctx(long, slice(long, 50, 60), scans), 103)).toBe(2);
  expect(calls[1]).toEqual(["start", 102]);
  // dentro la finestra nessuna chiamata
  expect(await stepUp(ctx(long, slice(long, 0, long.length), scans), 105)).toBe(103);
  expect(calls).toHaveLength(2);
});

test("progress updates fwd and back even when the scan is then cancelled", async () => {
  const cancelled: Scans = {
    next: async (_from, onProgress) => { onProgress(50); throw CANCELLED; },
    start: async (_at, onProgress) => { onProgress(40); throw CANCELLED; },
  };
  const c = ctx(long, slice(long, 0, 10), cancelled);
  await expect(stepDown(c, 2, 1)).rejects.toBe(CANCELLED);
  expect(c.cache.fwd.get(2)).toBe(50);
  expect(c.cache.next.has(2)).toBe(false);
  expect(c.inflight.size).toBe(0);
  const u = ctx(long, slice(long, 60, 20), cancelled);
  await expect(stepUp(u, 103)).rejects.toBe(CANCELLED);
  expect(u.cache.back.get(103)).toBe(40);
  // la scansione ripetuta riparte dal punto più avanzato
  const { calls, scans } = backend(long);
  c.scans = scans;
  expect(await nextStart(c, 2)).toBe(103);
  expect(calls).toEqual([["next", 50]]);
});

test("a result arriving after an encoding change is not cached", async () => {
  let resolve = (_: number) => {};
  const scans: Scans = { next: () => new Promise((r) => (resolve = r)), start: async () => 0 };
  const c = ctx(long, slice(long, 0, 10), scans);
  const p = nextStart(c, 2);
  lineCache(`nav${tabs}`, "utf-16le"); // la stessa cache, reimpostata sul posto
  resolve(103);
  expect(await p).toBe(103);
  expect(c.cache.next.size).toBe(0);
});

test("nextStart: two requests for the same line share one scan", async () => {
  const { calls, scans } = backend(long);
  const c = ctx(long, slice(long, 0, 10), scans);
  expect(await Promise.all([nextStart(c, 2), nextStart(c, 2)])).toEqual([103, 103]);
  expect(calls).toHaveLength(1);
});

test("stepDown stops when the last line of the file is the last of the viewport", async () => {
  for (const text of ["a\nb\nc\n", "a\nb\nc"]) {
    const data = enc(text);
    const c = ctx(data, slice(data, 0, data.length), backend(data).scans);
    expect(await stepDown(c, 0, 2)).toBe(2);
    expect(await stepDown(c, 2, 2)).toBeNull();
    expect(await stepDown(c, 0, 5)).toBeNull();
    expect(await lastPageTop(c, 2)).toBe(2); // Ctrl+Fine arriva allo stesso punto
    expect(await lastPageTop(c, 5)).toBe(0);
  }
  const empty = enc("");
  expect(await lastPageTop(ctx(empty, slice(empty, 0, 0), backend(empty).scans), 3)).toBe(0);
});

test("lastPageTop from a window near the end climbs over a long line with the backend", async () => {
  const { calls, scans } = backend(long);
  expect(await lastPageTop(ctx(long, slice(long, 90, 17), scans), 3)).toBe(2);
  expect(calls).toEqual([["start", 102]]);
});

test("BOM: the first line starts after it and top never goes below it", async () => {
  const data = new Uint8Array([0xef, 0xbb, 0xbf, ...enc("a\nb\n")]);
  const c = ctx(data, slice(data, 0, data.length), backend(data).scans, 3);
  expect(await stepUp(c, 5)).toBe(3);
  expect(await stepUp(c, 3)).toBeNull();
  expect(await lineAt(c, 0)).toBe(3);
  expect(await lineAt(c, 3)).toBe(3);
  expect(await lineAt(c, 4)).toBe(3);
  expect(await lineAt(c, 6)).toBe(5);
  expect(await lastPageTop(c, 5)).toBe(3);
});

test("lineAt: inside the window no backend, outside the line start from the backend", async () => {
  const { calls, scans } = backend(long);
  expect(await lineAt(ctx(long, slice(long, 0, long.length), scans), 50)).toBe(2);
  expect(await lineAt(ctx(long, slice(long, 40, 20), scans), 50)).toBe(2);
  expect(calls).toEqual([["start", 50]]);
});

test("visibleLines: window first, then the cached next, without the empty line after the last LF", () => {
  const { scans } = backend(long);
  const c = ctx(long, slice(long, 0, 10), scans);
  expect(visibleLines(c, 0, 5)).toEqual([
    { start: 0, end: 1, next: 2 },
    { start: 2, end: null, next: null },
  ]);
  c.cache.next.set(2, 103);
  expect(visibleLines(c, 0, 5).map((l) => [l.start, l.next])).toEqual([[0, 2], [2, 103], [103, null]]);
  expect(visibleLines(c, 0, 2).map((l) => l.start)).toEqual([0, 2]);
  const full = ctx(long, slice(long, 0, long.length), scans);
  expect(visibleLines(full, 103, 5)).toEqual([
    { start: 103, end: 104, next: 105 },
    { start: 105, end: 106, next: 107 },
  ]);
});
