import { expect, test, vi } from "vitest";

vi.mock("../../ipc/api");
import { endRows, layoutAt, maxScroll } from "./TextPane";

const bytes = (s: string) => new TextEncoder().encode(s);
const lines = (n: number) => bytes(Array.from({ length: n }, (_, i) => `riga ${i}\n`).join(""));

test("layoutAt: complete lines from top, the last of a partial window is dropped", () => {
  const buf = lines(100);
  const lay = layoutAt({ base: 0, buf, eof: false }, 0, 10, 1, false)!;
  expect(lay.all).toBe(false);
  expect(lay.starts.slice(0, 3)).toEqual([0, 7, 14]);
  expect(lay.n).toBe(lay.starts.length - 1);
  expect(lay.n).toBeGreaterThan(10);
});

test("layoutAt: needs a new read when the window does not cover top or has too few lines", () => {
  const buf = lines(5);
  expect(layoutAt(null, 0, 10, 1, false)).toBeNull();
  expect(layoutAt({ base: 100, buf, eof: true }, 0, 10, 1, false)).toBeNull(); // top prima della finestra
  expect(layoutAt({ base: 0, buf, eof: false }, 0, 10, 1, false)).toBeNull(); // poche righe e il file continua
});

test("layoutAt: at the end of the file every line is complete, even with few of them", () => {
  const lay = layoutAt({ base: 0, buf: lines(5), eof: true }, 0, 10, 1, false)!;
  expect(lay).toMatchObject({ all: true, n: 5 });
});

test("layoutAt: UTF-16LE lines split on the two-byte LF", () => {
  const u16 = new Uint8Array([0x61, 0, 0x0a, 0, 0x62, 0, 0x0a, 0]);
  const lay = layoutAt({ base: 0, buf: u16, eof: true }, 0, 1, 2, false)!;
  expect(lay.starts).toEqual([0, 4]);
});

test("maxScroll/endRows: without wrap the last page is full, with wrap the last logical line is reachable", () => {
  const eof = { n: 100, all: true };
  expect(maxScroll(eof, 30, false)).toBe(70); // l'ultima pagina resta piena
  expect(maxScroll(eof, 30, true)).toBe(99); // l'ultima riga arriva in cima
  expect(maxScroll({ n: 5, all: true }, 30, true)).toBe(4);
  expect(maxScroll({ n: 5, all: true }, 30, false)).toBe(0);
  expect(maxScroll({ n: 100, all: false }, 30, true)).toBe(99); // il file continua: invariato
  expect(maxScroll({ n: 100, all: false }, 30, false)).toBe(99);
  expect(endRows(30, false)).toBe(30);
  expect(endRows(30, true)).toBe(1);
});
