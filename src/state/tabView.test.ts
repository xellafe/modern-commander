import { describe, expect, it } from "vitest";
import type { Entry } from "../ipc/types";
import {
  back,
  buildRows,
  emptyView,
  finishSizing,
  forward,
  hasSize,
  invertMarks,
  jumpTo,
  markRange,
  navigateTo,
  parentPath,
  quickSearch,
  selectByMask,
  sizable,
  startSizing,
  toggleMark,
  type Row,
  type TabView,
} from "./tabView";

const f = (name: string, size = 0, attributes = 0, modifiedMs = 0): Entry => ({
  name, isDir: false, size, modifiedMs, createdMs: 0, accessedMs: 0, attributes, isSymlink: false,
});
const d = (name: string, attributes = 0): Entry => ({
  name, isDir: true, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes, isSymlink: false,
});
const opts = { showHidden: false, showSystem: false, filter: "" };
const sortName = { key: "name" as const, desc: false };
const rowsNamed = (names: string[]): Row[] =>
  names.map((n) => (n === ".." ? { ...d(".."), isParent: true } : f(n)));
const view = (names: string[]): TabView => ({ ...emptyView("C:\\x"), rows: rowsNamed(names) });
const names = (rows: Row[]) => rows.map((r) => r.name);

describe("buildRows", () => {
  it("sorts by created, accessed and type", () => {
    const t = (name: string, createdMs: number, accessedMs: number) => ({ ...f(name), createdMs, accessedMs });
    const e = [t("a.txt", 3, 1), t("b.png", 1, 2), t("c.zzz", 2, 3), d("dir")];
    const by = (key: "created" | "accessed" | "type", desc = false, typeNames?: Record<string, string>) =>
      names(buildRows(e, "C:\\", { key, desc }, { ...opts, typeNames }));
    expect(by("created")).toEqual(["dir", "b.png", "c.zzz", "a.txt"]);
    expect(by("created", true)).toEqual(["dir", "a.txt", "c.zzz", "b.png"]);
    expect(by("accessed")).toEqual(["dir", "a.txt", "b.png", "c.zzz"]);
    expect(by("accessed", true)).toEqual(["dir", "c.zzz", "b.png", "a.txt"]);
    // un'estensione senza testo vale stringa vuota e va in testa; le cartelle restano prima
    expect(by("type", false, { txt: "Documento di testo", png: "Immagine PNG", "/": "Cartella di file" })).toEqual(["dir", "c.zzz", "a.txt", "b.png"]);
    const odd = [f("x.constructor"), f("y.txt")];
    expect(names(buildRows(odd, "C:\\", { key: "type", desc: false }, { ...opts, typeNames: { txt: "Documento" } }))).toEqual(["x.constructor", "y.txt"]);
  });

  it("parent first, dirs before files, then sort", () => {
    const e = [f("b.txt", 2), d("zdir"), f("a.md", 9), d("adir")];
    const rows = buildRows(e, "C:\\x", { key: "size", desc: true }, opts);
    expect(names(rows)).toEqual(["..", "adir", "zdir", "a.md", "b.txt"]);
  });

  it("riga '..' con i valori fissi", () => {
    expect(buildRows([], "C:\\x", sortName, opts)[0]).toEqual({
      name: "..", isDir: true, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false, isParent: true,
    });
  });

  it("no parent row at drive root", () => {
    expect(buildRows([], "C:\\", sortName, opts)[0]).toBeUndefined();
  });

  it("hidden/system filtered unless enabled", () => {
    const e = [f("h", 1, 0x2), f("s", 1, 0x4), f("n", 1)];
    expect(names(buildRows(e, "C:\\x", sortName, opts))).toEqual(["..", "n"]);
    expect(names(buildRows(e, "C:\\x", sortName, { ...opts, showHidden: true }))).toEqual(["..", "h", "n"]);
    expect(names(buildRows(e, "C:\\x", sortName, { ...opts, showSystem: true }))).toEqual(["..", "n", "s"]);
  });

  it("il filtro riguarda solo i file", () => {
    const e = [f("a.txt"), f("b.md"), d("docs")];
    expect(names(buildRows(e, "C:\\", sortName, { ...opts, filter: "*.txt" }))).toEqual(["docs", "a.txt"]);
  });

  it("nome: ordine numerico e case-insensitive, desc inverte", () => {
    const e = [f("f10"), f("F2"), f("f1")];
    expect(names(buildRows(e, "C:\\", sortName, opts))).toEqual(["f1", "F2", "f10"]);
    expect(names(buildRows(e, "C:\\", { key: "name", desc: true }, opts))).toEqual(["f10", "F2", "f1"]);
  });

  it("ext: a parità di estensione ordina per nome, anche in desc", () => {
    const e = [f("b.txt"), f("a.txt"), f("c.md"), f(".gitignore"), f("noext")];
    expect(names(buildRows(e, "C:\\", { key: "ext", desc: false }, opts)))
      .toEqual([".gitignore", "noext", "c.md", "a.txt", "b.txt"]);
    expect(names(buildRows(e, "C:\\", { key: "ext", desc: true }, opts)))
      .toEqual(["a.txt", "b.txt", "c.md", ".gitignore", "noext"]);
  });

  it("date e size; le cartelle usano dirSizes", () => {
    const e = [f("old", 1, 0, 10), f("new", 5, 0, 20)];
    expect(names(buildRows(e, "C:\\", { key: "date", desc: true }, opts))).toEqual(["new", "old"]);
    const dirs = [d("a"), d("b")];
    const rows = buildRows(dirs, "C:\\", { key: "size", desc: true }, { ...opts, dirSizes: { b: 100 } });
    expect(names(rows)).toEqual(["b", "a"]);
  });

  it("una cartella chiamata come una proprietà di Object non rompe l'ordinamento per dimensione", () => {
    const dirs = [d("constructor"), d("toString"), d("b")];
    const rows = buildRows(dirs, "C:\\", { key: "size", desc: true }, { ...opts, dirSizes: { b: 100 } });
    expect(names(rows)).toEqual(["b", "constructor", "toString"]);
    // e quando la dimensione è davvero stata calcolata
    const own = buildRows(dirs, "C:\\", { key: "size", desc: false }, { ...opts, dirSizes: { b: 100, constructor: 50 } });
    expect(names(own)).toEqual(["toString", "constructor", "b"]);
  });

  it("none mantiene l'ordine del listing (con blocchi)", () => {
    const e = [f("z"), d("y"), f("a")];
    expect(names(buildRows(e, "C:\\", { key: "none", desc: false }, opts))).toEqual(["y", "z", "a"]);
  });
});

describe("dimensione delle cartelle", () => {
  const size = (bytes: number, skipped = 0) => ({ bytes, files: 1, dirs: 0, skipped });

  it("sizable: solo cartelle vere, non '..' né unità", () => {
    expect(sizable(d("x"))).toBe(true);
    expect(sizable(f("x"))).toBe(false);
    expect(sizable({ ...d(".."), isParent: true })).toBe(false);
    expect(sizable({ ...d("C:\\"), drive: { root: "C:\\", label: "", kind: "fixed", total: 0, free: 0, ready: true } })).toBe(false);
  });

  it("startSizing / finishSizing: il risultato sostituisce il calcolo in corso", () => {
    let v = startSizing(emptyView("C:\\"), ["a", "b"]);
    expect([...v.sizing]).toEqual(["a", "b"]);
    v = finishSizing(v, "a", size(7, 2));
    expect([...v.sizing]).toEqual(["b"]);
    expect(v.dirSizes).toEqual({ a: 7 });
    expect(v.skipped).toEqual({ a: 2 });
    // ricalcolo senza saltati: il marcatore sparisce
    v = finishSizing(startSizing(v, ["a"]), "a", size(9));
    expect([v.dirSizes, v.skipped]).toEqual([{ a: 9 }, {}]);
  });

  it("finishSizing con null (annullato o errore) toglie solo il calcolo in corso", () => {
    const v = finishSizing(startSizing(emptyView("C:\\"), ["a"]), "a", null);
    expect([v.sizing.size, v.dirSizes]).toEqual([0, {}]);
  });

  it("un nome come '__proto__' resta una proprietà propria", () => {
    const v = finishSizing(emptyView("C:\\"), "__proto__", size(3));
    expect(Object.keys(v.dirSizes)).toEqual(["__proto__"]);
    expect(hasSize(v.dirSizes, "__proto__")).toBe(true);
    expect(hasSize(v.dirSizes, "constructor")).toBe(false);
  });
});

describe("parentPath", () => {
  it("casi del ruling", () => {
    expect(parentPath("C:\\x\\y")).toBe("C:\\x");
    expect(parentPath("C:\\x")).toBe("C:\\");
    expect(parentPath("C:\\")).toBeNull();
    expect(parentPath("C:\\x\\y\\")).toBe("C:\\x");
    expect(parentPath("\\\\srv\\share\\x")).toBe("\\\\srv\\share");
    expect(parentPath("\\\\srv\\share\\x\\y")).toBe("\\\\srv\\share\\x");
    expect(parentPath("\\\\srv\\share")).toBeNull();
    expect(parentPath("\\\\srv\\share\\")).toBeNull();
  });
});

describe("marcatura", () => {
  it("parent row is never marked", () => {
    expect(toggleMark(view(["..", "a"]), "..").marked.size).toBe(0);
  });

  it("toggleMark marca e smarca senza mutare", () => {
    const v = view(["..", "a"]);
    const m = toggleMark(v, "a");
    expect([...m.marked]).toEqual(["a"]);
    expect(v.marked.size).toBe(0);
    expect(toggleMark(m, "a").marked.size).toBe(0);
  });

  it("markRange include gli estremi, in qualsiasi ordine, salta '..'", () => {
    const v = view(["..", "a", "b", "c", "d"]);
    expect([...markRange(v, 0, 2).marked]).toEqual(["a", "b"]);
    expect([...markRange(v, 3, 1).marked].sort()).toEqual(["a", "b", "c"]);
  });

  it("invertMarks", () => {
    const v = toggleMark(view(["..", "a", "b"]), "a");
    expect([...invertMarks(v).marked]).toEqual(["b"]);
  });

  it("selectByMask: solo file, '*.*' include anche le cartelle", () => {
    const v: TabView = { ...emptyView("C:\\x"), rows: [{ ...d(".."), isParent: true }, d("dir"), f("a.txt"), f("b.md")] };
    expect([...selectByMask(v, "*.txt", true).marked]).toEqual(["a.txt"]);
    expect([...selectByMask(v, "*", true).marked].sort()).toEqual(["a.txt", "b.md", "dir"]);
    expect([...selectByMask(v, "*.*", true).marked].sort()).toEqual(["a.txt", "b.md", "dir"]);
    const all = selectByMask(v, "*", true);
    expect([...selectByMask(all, "*.md", false).marked].sort()).toEqual(["a.txt", "dir"]);
  });
});

describe("history", () => {
  it("history truncates forward on navigate", () => {
    let v = navigateTo(navigateTo(emptyView("C:\\"), "C:\\a"), "C:\\b");
    v = navigateTo(back(v), "C:\\c");
    expect(v.history).toEqual(["C:\\", "C:\\a", "C:\\c"]);
    expect(v.historyIndex).toBe(2);
  });

  it("navigateTo azzera cursore, marcature, filtro e dirSizes", () => {
    const v0: TabView = { ...view(["..", "a"]), cursor: 1, marked: new Set(["a"]), filter: "*.x", dirSizes: { a: 1 } };
    const v = navigateTo(v0, "C:\\y");
    expect([v.cursor, v.marked.size, v.filter, v.dirSizes]).toEqual([0, 0, "", {}]);
  });

  it("navigateTo azzera anche i calcoli di dimensione in corso e i saltati", () => {
    const v0 = { ...emptyView("C:\\x"), sizing: new Set(["a"]), skipped: { a: 2 } };
    const v = navigateTo(v0, "C:\\y");
    expect([v.sizing.size, v.skipped]).toEqual([0, {}]);
  });

  it("stesso percorso: la history non cambia", () => {
    expect(navigateTo(emptyView("C:\\"), "C:\\").history).toEqual(["C:\\"]);
  });

  it("back/forward agli estremi sono no-op, altrimenti si spostano", () => {
    const v = navigateTo(emptyView("C:\\"), "C:\\a");
    expect(forward(v)).toBe(v);
    const b = back(v);
    expect(b.historyIndex).toBe(0);
    expect(back(b)).toBe(b);
    expect(forward(b).historyIndex).toBe(1);
  });

  it("jumpTo salta a un indice qualsiasi senza toccare la history", () => {
    const v = { ...navigateTo(navigateTo(emptyView("C:\\"), "C:\\a"), "C:\\b"), cursor: 3 };
    const j = jumpTo(v, 0);
    expect([j.historyIndex, j.cursor, j.history.length]).toEqual([0, 0, 3]);
    expect(jumpTo(v, 9)).toBe(v);
  });
});

describe("quickSearch", () => {
  it("quickSearch wraps", () => {
    expect(quickSearch(rowsNamed(["..", "alpha", "beta", "abc"]), "a", 2)).toBe(3);
    expect(quickSearch(rowsNamed(["..", "alpha", "beta", "xyz"]), "a", 2)).toBe(1);
  });

  it("from inclusivo, case-insensitive, -1 se nulla o prefisso vuoto, '..' mai", () => {
    const rows = rowsNamed(["..", "Alpha", "beta"]);
    expect(quickSearch(rows, "AL", 1)).toBe(1);
    expect(quickSearch(rows, "z", 0)).toBe(-1);
    expect(quickSearch(rows, "", 0)).toBe(-1);
    expect(quickSearch(rows, "..", 0)).toBe(-1);
  });
});
