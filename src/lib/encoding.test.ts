import { expect, test } from "vitest";
import { codeUnit, detectEncoding, ENCODINGS } from "./encoding";

const enc = (s: string) => new TextEncoder().encode(s);

test("encoding detection", () => {
  const W = "windows-1252";
  expect(detectEncoding(new Uint8Array([0xef, 0xbb, 0xbf, 0x41]), W, true)).toEqual({ encoding: "utf-8", bom: 3 });
  expect(detectEncoding(new Uint8Array([0xfe, 0xff, 0, 0x41]), W, true)).toEqual({ encoding: "utf-16be", bom: 2 });
  expect(detectEncoding(enc("perché"), W, true).encoding).toBe("utf-8");
  expect(detectEncoding(new Uint8Array([0x70, 0xe8, 0x20]), W, false).encoding).toBe(W);   // "pè " in ANSI
  expect(detectEncoding(enc("perché").slice(0, 6), W, false).encoding).toBe("utf-8");      // é tagliata dal campione
  expect(detectEncoding(new Uint8Array([0x70, 0xe8]), W, true).encoding).toBe(W);          // file intero che finisce con "è" ANSI
});

test("encoding detection: BOM UTF-16LE, ASCII puro, campione vuoto", () => {
  const W = "windows-1252";
  expect(detectEncoding(new Uint8Array([0xff, 0xfe, 0x41, 0]), W, false)).toEqual({ encoding: "utf-16le", bom: 2 });
  expect(detectEncoding(enc("abc"), W, true)).toEqual({ encoding: "utf-8", bom: 0 });
  expect(detectEncoding(new Uint8Array(0), W, true)).toEqual({ encoding: "utf-8", bom: 0 });
});

test("codeUnit e ENCODINGS", () => {
  expect(codeUnit("utf-16le")).toBe(2); expect(codeUnit("utf-16be")).toBe(2);
  expect(codeUnit("utf-8")).toBe(1); expect(codeUnit("shift_jis")).toBe(1);
  // ogni etichetta proposta deve essere decodificabile da TextDecoder (nessuna tabella scritta a mano)
  for (const e of ENCODINGS) expect(() => new TextDecoder(e)).not.toThrow();
  expect(ENCODINGS).toContain("windows-1250"); expect(ENCODINGS).toContain("windows-1258");
});
