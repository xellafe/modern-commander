import { describe, expect, it } from "vitest";
import { compileMask, matchMask, quickFilterMask } from "./mask";

describe("matchMask (semantica TC)", () => {
  it("copre i casi base", () => {
    expect(matchMask("a.TXT", "*.txt")).toBe(true);
    expect(matchMask("a.md", "*.txt;*.md")).toBe(true);
    expect(matchMask("a.md", "*.txt *.md")).toBe(true);
    expect(matchMask("a.bak.txt", "*.txt|*.bak.*")).toBe(false); // '|' esclude
    expect(matchMask("ab", "a?")).toBe(true);
    expect(matchMask("noext", "*.*")).toBe(true);
  });

  it("maschera vuota o solo esclusioni: include tutto", () => {
    expect(matchMask("x", "")).toBe(true);
    expect(matchMask("a.bak", "|*.bak")).toBe(false);
    expect(matchMask("a.txt", "|*.bak")).toBe(true);
  });

  it("escapa i metacaratteri regex", () => {
    expect(matchMask("a.txt", "a.txt")).toBe(true);
    expect(matchMask("abtxt", "a.txt")).toBe(false);
    expect(matchMask("a(1)+[x].txt", "a(1)+[x].*")).toBe(true);
  });

  it("'?' e '*' rispettano la lunghezza", () => {
    expect(matchMask("abc", "a?")).toBe(false);
    expect(matchMask("abc", "a*c")).toBe(true);
  });
});

describe("compileMask e quickFilterMask", () => {
  it("compileMask equivale a matchMask", () => {
    const m = compileMask("*.txt|a*");
    expect(m("b.TXT")).toBe(true);
    expect(m("a.txt")).toBe(false);
    expect(compileMask("")("x")).toBe(true);
  });

  it("il filtro rapido senza jolly cerca una sottostringa", () => {
    expect(quickFilterMask("ell")).toBe("*ell*");
    expect(quickFilterMask("*.md")).toBe("*.md");
    expect(quickFilterMask("")).toBe("");
  });

  it("il filtro rapido con spazi tratta ogni parola come sottostringa", () => {
    const names = ["my file.txt", "Nuovo documento di testo.txt", "altro.md"];
    const visible = (text: string) => names.filter(compileMask(quickFilterMask(text)));
    expect(visible("my file")).toContain("my file.txt");
    expect(visible("documento testo")).toEqual(["Nuovo documento di testo.txt"]);
    expect(quickFilterMask("  my ; file ")).toBe("*my*;*file*");
  });
});
