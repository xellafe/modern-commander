import { expect, test } from "vitest";
import { cssToHex, languageFor } from "./monaco";

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
