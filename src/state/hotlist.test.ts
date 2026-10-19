import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api", () => ({ readConfig: vi.fn(), writeConfig: vi.fn() }));
import { readConfig, writeConfig } from "../ipc/api";
import {
  addDir, addSubmenu, hotlistDirs, isInside, moveItem, parseHotlist, removeItem, renameItem, resolveDrop, useHotlist,
  type HotlistItem,
} from "./hotlist";

const dir = (id: string, path = `C:\\${id}`): HotlistItem => ({ type: "dir", id, name: id, path });
const sub = (id: string, items: HotlistItem[] = []): HotlistItem => ({ type: "submenu", id, name: id, items });
const ids = (l: HotlistItem[]): unknown[] => l.map((i) => (i.type === "submenu" ? [i.id, ids(i.items)] : i.id));

// a, S{ b, T{ c } }, d
const sample = (): HotlistItem[] => [dir("a"), sub("S", [dir("b"), sub("T", [dir("c")])]), dir("d")];

test("addDir appends at the root, or inside a nested submenu", () => {
  const root = addDir(sample(), "C:\\x", "x");
  expect(ids(root)).toEqual(["a", ["S", ["b", ["T", ["c"]]]], "d", root[3].id]);
  expect(root[3]).toMatchObject({ type: "dir", name: "x", path: "C:\\x" });

  const nested = addDir(sample(), "C:\\y", "y", "T");
  const t = ((nested[1] as { items: HotlistItem[] }).items[1] as { items: HotlistItem[] }).items;
  expect(t.map((i) => i.type === "dir" && i.path)).toEqual(["C:\\c", "C:\\y"]);
});

test("addDir with an unknown parent leaves the list unchanged; so does a dir used as parent", () => {
  const l = sample();
  expect(addDir(l, "C:\\x", "x", "nope")).toBe(l);
  expect(addDir(l, "C:\\x", "x", "a")).toBe(l);
});

test("addSubmenu creates an empty submenu with the given id", () => {
  const l = addSubmenu(sample(), "New", undefined, "N1");
  expect(l[3]).toEqual({ type: "submenu", id: "N1", name: "New", items: [] });
});

test("removeItem of a submenu removes its children too", () => {
  expect(ids(removeItem(sample(), "S"))).toEqual(["a", "d"]);
  expect(ids(removeItem(sample(), "c"))).toEqual(["a", ["S", ["b", ["T", []]]], "d"]);
});

test("renameItem renames dirs and submenus at any depth", () => {
  const l = renameItem(renameItem(sample(), "c", "Cee"), "S", "Ess");
  expect(l[1]).toMatchObject({ name: "Ess" });
  expect(((l[1] as { items: HotlistItem[] }).items[1] as { items: HotlistItem[] }).items[0]).toMatchObject({ name: "Cee", path: "C:\\c" });
});

test("moveItem moves from a submenu to the root at the given index", () => {
  expect(ids(moveItem(sample(), "b", null, 0))).toEqual(["b", "a", ["S", [["T", ["c"]]]], "d"]);
  expect(ids(moveItem(sample(), "c", null, 99))).toEqual(["a", ["S", ["b", ["T", []]]], "d", "c"]);
});

test("moveItem moves into a submenu; index counts without the moved item", () => {
  expect(ids(moveItem(sample(), "a", "T", 0))).toEqual([["S", ["b", ["T", ["a", "c"]]]], "d"]);
  expect(ids(moveItem(sample(), "a", null, 1))).toEqual([["S", ["b", ["T", ["c"]]]], "a", "d"]);
});

test("moveItem of a submenu into itself or a descendant leaves the list unchanged", () => {
  const l = sample();
  expect(moveItem(l, "S", "S", 0)).toBe(l);
  expect(moveItem(l, "S", "T", 0)).toBe(l);
  expect(moveItem(l, "nope", null, 0)).toBe(l);
  expect(moveItem(l, "a", "nope", 0)).toBe(l);
  expect(isInside(l, "S", "c")).toBe(true);
  expect(isInside(l, "T", "S")).toBe(false);
});

test("resolveDrop converts before/after/into to a parent and an index valid for moveItem", () => {
  const l = sample();
  expect(resolveDrop(l, "a", "d", "after")).toEqual({ toParentId: null, index: 2 });
  expect(resolveDrop(l, "d", "a", "before")).toEqual({ toParentId: null, index: 0 });
  expect(resolveDrop(l, "d", "b", "before")).toEqual({ toParentId: "S", index: 0 });
  expect(resolveDrop(l, "a", "T", "into")).toEqual({ toParentId: "T", index: 1 });
  expect(resolveDrop(l, "a", "b", "into")).toBeNull(); // b è una cartella, non un sottomenu
  expect(resolveDrop(l, "S", "c", "before")).toBeNull(); // dentro se stesso
  expect(resolveDrop(l, "a", "a", "before")).toBeNull();
  // stesso genitore, verso il basso: l'indice conta senza l'elemento spostato
  expect(ids(moveItem(l, "a", ...Object.values(resolveDrop(l, "a", "S", "after")!) as [null, number]))).toEqual([["S", ["b", ["T", ["c"]]]], "a", "d"]);
});

test("parseHotlist accepts well-formed lists and rejects everything else", () => {
  const good = [dir("a"), sub("S", [dir("b"), { type: "separator", id: "sep" }])];
  expect(parseHotlist(good)).toEqual(good);
  expect(parseHotlist([])).toEqual([]);
  expect(parseHotlist(null)).toBeNull();
  expect(parseHotlist({})).toBeNull();
  expect(parseHotlist([{ type: "dir", id: "a", name: "a" }])).toBeNull(); // manca path
  expect(parseHotlist([{ type: "weird", id: "a" }])).toBeNull();
  expect(parseHotlist([sub("S", [{ type: "dir", id: "b", name: 1, path: "x" } as never])])).toBeNull();
  expect(parseHotlist([dir("a"), dir("a")])).toBeNull(); // id duplicato
  expect(parseHotlist([sub("a", [dir("a")])])).toBeNull(); // id duplicato annidato
});

const read = vi.mocked(readConfig);
const write = vi.mocked(writeConfig);
beforeEach(() => {
  read.mockReset();
  write.mockReset();
  write.mockResolvedValue(undefined);
  useHotlist.setState({ items: [], loaded: false });
});

test("load: a valid file populates the list without writing", async () => {
  read.mockResolvedValue([dir("a")]);
  await useHotlist.getState().load();
  expect(useHotlist.getState().items).toEqual([dir("a")]);
  expect(write).not.toHaveBeenCalled();
});

test("load: a malformed or unreadable file starts empty and does not overwrite it", async () => {
  read.mockResolvedValue({ not: "a list" });
  await useHotlist.getState().load();
  expect(useHotlist.getState().items).toEqual([]);
  read.mockRejectedValue(new Error("boom"));
  await useHotlist.getState().load();
  expect(useHotlist.getState().items).toEqual([]);
  expect(useHotlist.getState().loaded).toBe(true);
  expect(write).not.toHaveBeenCalled();
});

test("changes are persisted in call order; a failed write does not block the next", async () => {
  const events: string[] = [];
  const delays = [30, 0];
  write.mockImplementation((_, v) => {
    const n = (v as HotlistItem[]).length;
    events.push(`start ${n}`);
    return new Promise((res, rej) => setTimeout(() => {
      events.push(`end ${n}`);
      return n === 1 ? rej(new Error("boom")) : res(undefined);
    }, delays.shift()));
  });
  const err = vi.spyOn(console, "error").mockImplementation(() => {});
  useHotlist.getState().addDir("C:\\a", "a");
  useHotlist.getState().addDir("C:\\b", "b");
  await vi.waitFor(() => expect(events).toEqual(["start 1", "end 1", "start 2", "end 2"]));
  expect(write.mock.calls.map((c) => (c[1] as HotlistItem[]).length)).toEqual([1, 2]);
  expect(write.mock.calls[0][0]).toBe("hotlist");
  expect(err).toHaveBeenCalledTimes(1);
  err.mockRestore();
});

test("hotlistDirs flattens submenus in menu order and skips separators", () => {
  const list: HotlistItem[] = [dir("A"), sub("S", [dir("B"), { type: "separator", id: "sep" }]), dir("C")];
  expect(hotlistDirs(list)).toEqual([
    { name: "A", path: "C:\\A" },
    { name: "B", path: "C:\\B" },
    { name: "C", path: "C:\\C" },
  ]);
  expect(hotlistDirs([])).toEqual([]);
});
