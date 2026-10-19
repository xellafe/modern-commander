import { expect, test } from "vitest";
import { lineUp, MAX_LINE, seekLine, splitLines } from "./textPaging";

const enc = (s: string) => new TextEncoder().encode(s);

test("splitLines", () => {
  expect(splitLines(enc("a\nbb\n\nc"), 0, 1, false)).toEqual([0, 2, 5, 6]);
  expect(splitLines(enc("x\ny"), 100, 1, false)).toEqual([100, 102]);
  expect(splitLines(new Uint8Array([0x61, 0, 0x0a, 0, 0x62, 0]), 0, 2, false)).toEqual([0, 4]);
  expect(splitLines(new Uint8Array(10000).fill(0x78), 0, 1, false)).toEqual([0, 4096, 8192]);
});

test("splitLines: UTF-16BE, LF a fine blocco, blocco vuoto, riga lunga in UTF-16", () => {
  expect(splitLines(new Uint8Array([0, 0x61, 0, 0x0a, 0, 0x62]), 0, 2, true)).toEqual([0, 4]);
  // 00 0A a cavallo di due unità (allineamento dispari) non è un LF
  expect(splitLines(new Uint8Array([0x01, 0x00, 0x0a, 0x61]), 0, 2, true)).toEqual([0]);
  expect(splitLines(enc("ab\n"), 0, 1, false)).toEqual([0]);   // l'inizio dopo l'LF finale è fuori dal blocco
  expect(splitLines(new Uint8Array(0), 40, 1, false)).toEqual([]);
  expect(splitLines(new Uint8Array(10000).fill(0x78), 8, 2, false)).toEqual([8, 4104, 8200]);
  expect(MAX_LINE).toBe(4096);
});

test("lineUp", () => {
  const t = enc("aa\nbbb\ncc");
  expect(lineUp(t.slice(0, 7), 0, 7, 1, false)).toBe(3);
  expect(lineUp(t.slice(0, 3), 0, 3, 1, false)).toBe(0);
  expect(lineUp(new Uint8Array(0), 0, 0, 1, false)).toBe(0);
  expect(lineUp(new Uint8Array(8192).fill(0x78), 0, 8192, 1, false)).toBe(4096);
  expect(lineUp(new Uint8Array(65536).fill(0x78), 1000, 66536, 1, false)).toBe(66536 - 4096);
});

test("lineUp: UTF-16, riga lunga dopo un LF, nessun LF con base piccola", () => {
  const le = new Uint8Array([0x61, 0, 0x0a, 0, 0x62, 0, 0x0a, 0]);
  expect(lineUp(le, 0, 8, 2, false)).toBe(4);
  const be = new Uint8Array([0, 0x61, 0, 0x0a, 0, 0x62, 0, 0x0a]);
  expect(lineUp(be, 0, 8, 2, true)).toBe(4);
  // "\n" + 5000 byte + "\n": la riga sopra top è lunga, vale l'ultimo spezzone che inizia prima di top
  const long = new Uint8Array(5002).fill(0x78); long[0] = 0x0a; long[5001] = 0x0a;
  expect(lineUp(long, 0, 5002, 1, false)).toBe(1 + 4096);
  // meno di MAX_LINE senza LF e base > 0: si resta a base
  expect(lineUp(new Uint8Array(100).fill(0x78), 50, 150, 1, false)).toBe(50);
});

test("seekLine", () => {
  expect(seekLine(enc("zz\nnext"), 50, 1, false)).toBe(53);
  expect(seekLine(new Uint8Array(5000).fill(0x78), 51, 2, false)).toBe(50);
  expect(seekLine(enc("abc"), 0, 1, false)).toBe(0);
});

test("seekLine: UTF-16, LF oltre MAX_LINE ignorato", () => {
  expect(seekLine(new Uint8Array([0x61, 0, 0x0a, 0, 0x62, 0]), 100, 2, false)).toBe(104);
  expect(seekLine(new Uint8Array([0, 0x61, 0, 0x0a, 0, 0x62]), 100, 2, true)).toBe(104);
  const far = new Uint8Array(6000).fill(0x78); far[5000] = 0x0a;
  expect(seekLine(far, 10, 1, false)).toBe(10);
});
