import { expect, test } from "vitest";
import { isLineStart, mergeMarks, nearestMark, prevLineStart, queueNav, renderSlice, wholeLines } from "./longLines";
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
