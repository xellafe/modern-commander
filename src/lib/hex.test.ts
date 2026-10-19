import { expect, test } from "vitest";
import { hexRow, offsetDigits } from "./hex";

test("hex rows", () => {
  const full = hexRow(Uint8Array.from({ length: 16 }, (_, i) => i), 0, 8);
  expect(full.hex).toBe("00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F");
  const r = hexRow(new Uint8Array([0x41, 0x00, 0xff]), 16, 8);
  expect(r).toMatchObject({ offset: "00000010", ascii: "A.." });
  expect(r.hex.trimEnd()).toBe("41 00 FF"); expect(r.hex).toHaveLength(full.hex.length);
  expect(offsetDigits(1000)).toBe(8); expect(offsetDigits(2 ** 36)).toBe(10);
});

test("hex rows: limiti ASCII e offset maiuscolo", () => {
  expect(hexRow(new Uint8Array([0x1f, 0x20, 0x7e, 0x7f]), 0xabc, 4)).toMatchObject({ offset: "0ABC", ascii: ". ~." });
  expect(hexRow(new Uint8Array(0), 0, 8).hex).toHaveLength(48);
});
