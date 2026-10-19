import { expect, test } from "vitest";
import { lineCache } from "../state/lineCache";
import { LINE_END } from "../ipc/api";
import type { Mark } from "../ipc/types";
import {
  columnHits, isLineStart, lastPageTop, lineAt, lineExtent, maxLeft, mergeMarks, nearestMark, nextStart, prevLineStart,
  queueNav, renderSlice, stepDown, stepUp, visibleLines, wholeLines, type Columns, type NavCtx, type Scans,
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

test("nextStart restarts from the furthest known point, fwd or the window end", async () => {
  const { calls, scans } = backend(long);
  const c = ctx(long, slice(long, 0, 60), scans);
  c.cache.fwd.set(2, 20); // meno avanti della fine della finestra
  expect(await nextStart(c, 2)).toBe(103);
  expect(calls).toEqual([["next", 60]]);
});

test("a cancelled scan does not drop the newer scan of the same line", async () => {
  const pending: { reject: (e: unknown) => void; resolve: (n: number) => void }[] = [];
  const scans: Scans = {
    next: () => new Promise((resolve, reject) => pending.push({ resolve, reject })),
    start: async () => 0,
  };
  const c = ctx(long, slice(long, 0, 10), scans);
  const first = nextStart(c, 2);
  c.inflight.clear(); // come `cancel()` nel pannello
  const second = nextStart(c, 2);
  pending[0].reject(CANCELLED);
  await expect(first).rejects.toBe(CANCELLED);
  expect(c.inflight.get(2)).toBe(second);
  pending[1].resolve(103);
  expect(await second).toBe(103);
  expect(c.inflight.size).toBe(0);
});

test("BOM: the line above found by the backend is cached under the real first line start", async () => {
  const data = new Uint8Array([0xef, 0xbb, 0xbf, ...enc("x".repeat(100) + "\nb\n")]);
  const { scans } = backend(data);
  const c = ctx(data, slice(data, 50, 60), scans, 3);
  expect(await stepUp(c, 104)).toBe(3);
  expect(c.cache.next.get(3)).toBe(104);
  expect(c.cache.next.has(0)).toBe(false);
});

test("lineAt keeps the backward progress and restarts from it", async () => {
  const cancelled: Scans = { next: async () => 0, start: async (_at, onProgress) => { onProgress(30); throw CANCELLED; } };
  const c = ctx(long, slice(long, 40, 20), cancelled);
  await expect(lineAt(c, 80)).rejects.toBe(CANCELLED);
  expect(c.cache.back.get(80)).toBe(30);
  const { calls, scans } = backend(long);
  c.scans = scans;
  expect(await lineAt(c, 80)).toBe(2);
  expect(calls).toEqual([["start", 30]]);
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

test("columnHits: one request for all rows, each from its nearest mark; progress and hits become marks, eol widths", async () => {
  const cache = lineCache(`col${++tabs}`, "utf-8");
  cache.marks.set(0, [{ offset: 1 << 20, column: 900_000 }, { offset: 2 << 20, column: 1_900_000 }]);
  const lines = [{ start: 0, end: null, next: null }, { start: 500, end: 550, next: 551 }];
  const calls: [Mark[], number][] = [];
  const columns: Columns = async (ls, from, target, onProgress) => {
    calls.push([from, target]);
    expect(ls).toEqual(lines);
    onProgress({ index: 0, mark: { offset: 3 << 20, column: 2_900_000 } });
    return [{ offset: 5_000_000, column: 3_000_000, eol: false }, { offset: 550, column: 50, eol: true }];
  };
  expect(await columnHits(cache, lines, 3_000_000, columns)).toEqual([
    { offset: 5_000_000, column: 3_000_000, eol: false },
    { offset: 550, column: 50, eol: true },
  ]);
  expect(calls).toEqual([[[{ offset: 2 << 20, column: 1_900_000 }, { offset: 500, column: 0 }], 3_000_000]]);
  expect(cache.marks.get(0)!.map((m) => m.column)).toEqual([900_000, 1_900_000, 2_900_000, 3_000_000]);
  expect(cache.widths.get(500)).toBe(50);
  expect(cache.marks.has(500)).toBe(false);
});

test("columnHits: a row of known width shorter than the target does not go to the backend", async () => {
  const cache = lineCache(`col${++tabs}`, "utf-8");
  cache.widths.set(0, 40);
  const lines = [{ start: 0, end: 45, next: 46 }, { start: 46, end: 300, next: 301 }];
  const calls: Mark[][] = [];
  const columns: Columns = async (ls, from) => {
    calls.push(from);
    expect(ls).toEqual([lines[1]]);
    return [{ offset: 146, column: 100, eol: false }];
  };
  const hits = await columnHits(cache, lines, 100, columns);
  expect(hits).toEqual([{ offset: 45, column: 40, eol: true }, { offset: 146, column: 100, eol: false }]);
  expect(calls).toEqual([[{ offset: 46, column: 0 }]]);
  // Fine riga: tutte le larghezze note, nessuna richiesta.
  cache.widths.set(46, 254);
  expect((await columnHits(cache, lines, LINE_END, columns)).map((h) => h.column)).toEqual([40, 254]);
  expect(calls.length).toBe(1);
  // Senza fine nota l'offset della fine riga non si conosce: la riga va al backend anche con la larghezza nota.
  const far = [{ start: 400, end: null, next: null }];
  cache.widths.set(400, 10);
  const r = await columnHits(cache, far, 100, async () => [{ offset: 411, column: 10, eol: true }]);
  expect(r).toEqual([{ offset: 411, column: 10, eol: true }]);
});

test("columnHits: a hit close to a known point is not stored, so ← → do not pile up marks", async () => {
  const cache = lineCache(`col${++tabs}`, "utf-8");
  const lines = [{ start: 0, end: null, next: null }];
  let at = 0;
  const columns: Columns = async (_ls, _from, target) => [{ offset: target + at, column: target, eol: false }];
  await columnHits(cache, lines, 10, columns); // vicino all'inizio riga
  expect(cache.marks.get(0)).toBeUndefined();
  at = 1 << 20;
  await columnHits(cache, lines, 10, columns);
  await columnHits(cache, lines, 11, columns); // a un carattere dal punto appena salvato
  expect(cache.marks.get(0)).toEqual([{ offset: (1 << 20) + 10, column: 10 }]);
});

test("columnHits: results arriving after an encoding change are not cached", async () => {
  const id = `col${++tabs}`;
  const cache = lineCache(id, "utf-8");
  const lines = [{ start: 0, end: null, next: null }, { start: 10, end: 20, next: 21 }];
  const columns: Columns = async (_ls, _from, _target, onProgress) => {
    lineCache(id, "windows-1252"); // la stessa cache, reimpostata sul posto
    onProgress({ index: 0, mark: { offset: 3 << 20, column: 3 << 20 } });
    return [{ offset: 5 << 20, column: 5 << 20, eol: false }, { offset: 20, column: 10, eol: true }];
  };
  await columnHits(cache, lines, 5 << 20, columns);
  expect(cache.marks.size).toBe(0);
  expect(cache.widths.size).toBe(0);
});

test("lineExtent: exact widths when known, otherwise bytes (halved in UTF-16); maxLeft keeps the rail within 0…1", () => {
  const cache = lineCache(`col${++tabs}`, "utf-8");
  const lines = [{ start: 0, end: 30, next: 31 }, { start: 31, end: 50, next: 51 }];
  expect(lineExtent(lines, cache, 1000, 1)).toBe(30);
  cache.widths.set(31, 70); // tab: più colonne che byte
  expect(lineExtent(lines, cache, 1000, 1)).toBe(70);
  expect(lineExtent([{ start: 2, end: null, next: null }], cache, 1002, 2)).toBe(500); // fino a fine file
  expect(lineExtent([{ start: 0, end: null, next: 101 }], cache, 1000, 1)).toBe(101); // fino alla riga dopo, non a fine file
  expect(lineExtent([], cache, 1000, 1)).toBe(0);
  expect(maxLeft(70, 50)).toBe(21);
  expect(maxLeft(30, 50)).toBe(0);
  expect(21 / Math.max(1, maxLeft(70, 50))).toBe(1);
});
