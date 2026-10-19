import { expect, test } from "vitest";
import { cssToHex, languageFor, worthColorizing } from "./monaco";

const langs = [
  { id: "typescript", extensions: [".ts", ".tsx"] },
  { id: "markdown", extensions: [".md"] },
  { id: "plaintext" },
];

test("languageFor finds the language by extension, ignoring case", () => {
  expect(languageFor("A.TSX", langs)).toBe("typescript");
  expect(languageFor("notes.md", langs)).toBe("markdown");
  expect(languageFor("lib.d.ts", langs)).toBe("typescript"); // conta l'ultima estensione
  expect(languageFor("a.xyz", langs)).toBeNull();
  expect(languageFor("Makefile", langs)).toBeNull(); // senza estensione
});

test("cssToHex turns token values into the hex colors Monaco accepts", () => {
  expect(cssToHex("#0e1013")).toBe("#0e1013");
  expect(cssToHex(" #FFF ")).toBe("#ffffff");
  expect(cssToHex("rgba(224, 122, 79, 0.14)")).toBe("#e07a4f24");
  expect(cssToHex("rgb(43, 42, 39)")).toBe("#2b2a27");
  expect(cssToHex("var(--x)")).toBeUndefined();
  expect(cssToHex("")).toBeUndefined();
});

test("worthColorizing skips plain text, no language and very long lines, with a linear scan", () => {
  expect(worthColorizing("let a", "typescript")).toBe(true);
  expect(worthColorizing("let a", "plaintext")).toBe(false);
  expect(worthColorizing("let a", null)).toBe(false);
  expect(worthColorizing("x".repeat(2000) + "\ny", "typescript")).toBe(true); // 2000 ok, oltre no
  expect(worthColorizing("a\n" + "x".repeat(2001) + "\nb", "typescript")).toBe(false);
  expect(worthColorizing("x".repeat(2001), "typescript")).toBe(false); // ultima riga senza a capo
  expect(worthColorizing("", "typescript")).toBe(true);
  expect(worthColorizing("a\n".repeat(100_000), "typescript")).toBe(true);
});
