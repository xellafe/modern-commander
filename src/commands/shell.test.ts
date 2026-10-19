import { beforeAll, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { activeTab, useLayout } from "../state/layout";
import { useSettings } from "../state/settings";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { useUi } from "../state/ui";
import { dropOnGroup } from "./fileOps";
import { registerNavigationCommands } from "./navigation";
import { executeCommand } from "./registry";
import {
  classifyDrop, dragOut, isSelfDrop, pastDragThreshold, registerShellCommands, rowsAt, SHELL_COMMAND_IDS, shellPaths,
  suppressKeyboardContextMenu, toPhysical,
} from "./shell";

const f = (name: string, isDir = false): Entry => ({ name, isDir, size: 0, modifiedMs: 0, attributes: 0, isSymlink: false });
const view = () => activeTabContext().view;
const setCursor = (name: string) =>
  useTabViews.getState().update("t1", (v) => ({ ...v, cursor: v.rows.findIndex((r) => r.name === name) }));
const mark = (...names: string[]) => useTabViews.getState().update("t1", (v) => ({ ...v, marked: new Set(names) }));
const indexOf = (name: string) => view().rows.findIndex((r) => r.name === name);

beforeAll(() => {
  registerNavigationCommands();
  registerShellCommands();
});
beforeEach(async () => {
  vi.resetAllMocks();
  vi.mocked(api.watchDir).mockResolvedValue();
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1"], activeTabId: "t1" }],
    tabs: { t1: { id: "t1", path: "C:\\" } },
    activeGroupId: "g1",
    activationOrder: ["g1"],
  });
  useTabViews.setState({ views: {}, notice: null });
  useUi.setState({ dialog: null });
  vi.mocked(api.listDir).mockResolvedValueOnce([f("sub", true), f("a.txt"), f("b.txt"), f("c.pdf")]);
  await useTabViews.getState().load("t1", "C:\\x");
});

test("rowsAt: a marked row brings all marked rows, an unmarked one only itself, '..' nothing", () => {
  mark("a.txt", "c.pdf");
  expect(rowsAt(view(), indexOf("c.pdf")).map((r) => r.name)).toEqual(["a.txt", "c.pdf"]);
  expect(rowsAt(view(), indexOf("b.txt")).map((r) => r.name)).toEqual(["b.txt"]);
  expect(rowsAt(view(), indexOf(".."))).toEqual([]);
  expect(rowsAt(view(), 99)).toEqual([]);
});

test("shellPaths joins names, uses the drive root for drives and falls back to the folder", () => {
  expect(shellPaths("C:\\x", [{ ...f("a.txt") }])).toEqual(["C:\\x\\a.txt"]);
  expect(shellPaths("C:\\", [{ ...f("a.txt") }])).toEqual(["C:\\a.txt"]);
  const drive = { root: "D:\\", label: "", ready: true, free: 0, total: 0 } as never;
  expect(shellPaths("drives:", [{ ...f("D:\\", true), drive }])).toEqual(["D:\\"]);
  expect(shellPaths("C:\\x", [])).toEqual(["C:\\x"]);
  expect(shellPaths("drives:", [])).toEqual([]);
});

test("toPhysical scales client coordinates and rounds to integers", () => {
  expect(toPhysical(10, 20, 1)).toEqual({ x: 10, y: 20 });
  expect(toPhysical(10.4, 20.5, 1.5)).toEqual({ x: 16, y: 31 });
});

test("the drag starts only beyond 4 px", () => {
  expect(pastDragThreshold(4, 0)).toBe(false);
  expect(pastDragThreshold(3, 3)).toBe(true);
  expect(pastDragThreshold(0, -5)).toBe(true);
});

test("cm_Return on a file opens it with the default program", async () => {
  vi.mocked(api.openPath).mockResolvedValue();
  setCursor("c.pdf");
  await executeCommand("cm_Return");
  expect(api.openPath).toHaveBeenCalledWith("C:\\x\\c.pdf");
  expect(api.listDir).toHaveBeenCalledTimes(1); // nessuna navigazione
});

test("cm_List opens the cursor file in a viewer tab, cm_Edit uses the external editor; folders and '..' are ignored", async () => {
  vi.mocked(api.openWith).mockResolvedValue();
  useSettings.setState({ externalEditor: "code.exe" });
  setCursor("a.txt");
  await executeCommand("cm_List");
  expect(activeTab(useLayout.getState()).file).toEqual({ path: "C:\\x\\a.txt", mode: "viewer" });
  expect(useLayout.getState().groups[0].tabIds).toHaveLength(2);
  expect(api.openPath).not.toHaveBeenCalled(); // F3 non apre più programmi esterni
  useLayout.getState().activateTab("t1");
  await executeCommand("cm_Edit");
  expect(api.openWith).toHaveBeenCalledWith("code.exe", "C:\\x\\a.txt");
  for (const name of ["sub", ".."]) {
    setCursor(name);
    await executeCommand("cm_List");
    await executeCommand("cm_Edit");
  }
  expect(useLayout.getState().groups[0].tabIds).toHaveLength(2);
  expect(api.openWith).toHaveBeenCalledTimes(1);
});

test("cm_Properties uses the marked rows, or the current folder on '..'", async () => {
  vi.mocked(api.showProperties).mockResolvedValue();
  mark("a.txt", "b.txt");
  await executeCommand("cm_Properties");
  expect(api.showProperties).toHaveBeenLastCalledWith(["C:\\x\\a.txt", "C:\\x\\b.txt"]);
  mark();
  setCursor("..");
  await executeCommand("cm_Properties");
  expect(api.showProperties).toHaveBeenLastCalledWith(["C:\\x"]);
});

test("CANCELLED is ignored, other errors become a notice", async () => {
  setCursor("a.txt");
  vi.mocked(api.openPath).mockRejectedValueOnce({ code: "CANCELLED", path: null, detail: null });
  await executeCommand("cm_Return");
  expect(useTabViews.getState().notice).toBeNull();
  vi.mocked(api.openPath).mockRejectedValueOnce({ code: "ACCESS_DENIED", path: "C:\\x\\a.txt", detail: null });
  await executeCommand("cm_Return");
  expect(useTabViews.getState().notice).toEqual({ tabId: "t1", code: "ACCESS_DENIED", path: "C:\\x\\a.txt" });
});

const macrotask = () => new Promise((r) => setTimeout(r, 0));

test("drops are ignored while our own drag is running and for its paths right after", async () => {
  let finish!: () => void;
  vi.mocked(api.startDrag).mockReturnValue(new Promise<void>((r) => (finish = r)));
  vi.mocked(api.normalizePaths).mockImplementation(async (p) => p);
  mark("a.txt", "b.txt");
  const drag = dragOut("t1", "b.txt");
  expect(api.startDrag).toHaveBeenCalledWith(["C:\\x\\a.txt", "C:\\x\\b.txt"]);
  await dropOnGroup("g1", ["C:\\x\\a.txt"]);
  expect(useUi.getState().dialog).toBeNull();
  finish();
  await drag;
  await dropOnGroup("g1", ["C:\\y\\z.txt"]); // stesso macrotask: ancora ignorato
  expect(useUi.getState().dialog).toBeNull();
  await macrotask();
  await dropOnGroup("g1", ["C:\\x\\b.txt", "C:\\x\\a.txt"]); // l'insieme appena trascinato
  expect(useUi.getState().dialog).toBeNull();
  await dropOnGroup("g1", ["C:\\y\\z.txt"]);
  expect(useUi.getState().dialog?.kind).toBe("copyMove");
});

test("isSelfDrop: running drag, or the same path set (any order and case) within 2 s", () => {
  const last = { paths: ["C:\\x\\a.txt", "C:\\x\\b.txt"], at: 1000 };
  expect(isSelfDrop(["C:\\y"], { dragging: true, last: null }, 0)).toBe(true);
  expect(isSelfDrop(["C:\\y"], { dragging: false, last: null }, 0)).toBe(false);
  expect(isSelfDrop(["c:\\X\\B.TXT", "C:\\x\\a.txt"], { dragging: false, last }, 2999)).toBe(true);
  expect(isSelfDrop(["C:\\x\\a.txt", "C:\\x\\b.txt"], { dragging: false, last }, 3000)).toBe(false);
  expect(isSelfDrop(["C:\\x\\a.txt"], { dragging: false, last }, 1500)).toBe(false);
  expect(isSelfDrop(["C:\\x\\a.txt", "C:\\x\\c.txt"], { dragging: false, last }, 1500)).toBe(false);
});

test("classifyDrop: an external drop, our drag on its own group (ignored) or on another group", () => {
  const s = { dragging: true, last: null, origin: { groupId: "g1", from: "C:\\x" } };
  expect(classifyDrop(["C:\\y"], { dragging: false, last: null }, 0, "g1")).toEqual({ kind: "external" });
  expect(classifyDrop(["C:\\x\\a.txt"], s, 0, "g1")).toEqual({ kind: "ignore" });
  expect(classifyDrop(["C:\\x\\a.txt"], s, 0, "g2")).toEqual({ kind: "fromGroup", from: "C:\\x" });
  expect(classifyDrop(["C:\\x\\a.txt"], { dragging: true, last: null }, 0, "g2")).toEqual({ kind: "ignore" });
});

test("rows dragged onto another group open the copy dialog from the origin folder", async () => {
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1"], activeTabId: "t1" }, { id: "g2", tabIds: ["t2"], activeTabId: "t2" }],
    tabs: { t1: { id: "t1", path: "C:\\x" }, t2: { id: "t2", path: "D:\\y" } },
    activeGroupId: "g1",
    activationOrder: ["g1", "g2"],
  });
  let finish!: () => void;
  vi.mocked(api.startDrag).mockReturnValue(new Promise<void>((r) => (finish = r)));
  vi.mocked(api.normalizePaths).mockImplementation(async (p) => p);
  mark("a.txt", "b.txt");
  const drag = dragOut("t1", "a.txt");
  await dropOnGroup("g1", ["C:\\x\\a.txt", "C:\\x\\b.txt"]); // sullo stesso gruppo: niente
  expect(useUi.getState().dialog).toBeNull();
  await dropOnGroup("g2", ["C:\\x\\a.txt", "C:\\x\\b.txt"]);
  expect(useUi.getState().dialog).toEqual({
    kind: "copyMove", move: false, sources: ["C:\\x\\a.txt", "C:\\x\\b.txt"], from: "C:\\x", dest: "D:\\y",
    requireChange: false, targets: [], policy: "ask",
  });
  finish();
  await drag;
});

test("a failed start_drag shows a notice and releases the self-drag flag", async () => {
  vi.mocked(api.startDrag).mockRejectedValueOnce({ code: "NOT_FOUND", path: "C:\\x\\c.pdf", detail: null });
  vi.mocked(api.normalizePaths).mockImplementation(async (p) => p);
  await dragOut("t1", "c.pdf");
  expect(useTabViews.getState().notice).toEqual({ tabId: "t1", code: "NOT_FOUND", path: "C:\\x\\c.pdf" });
  await macrotask();
  await dropOnGroup("g1", ["C:\\y\\z.txt"]);
  expect(useUi.getState().dialog?.kind).toBe("copyMove");
});

test("drive rows and '..' are never dragged", async () => {
  vi.mocked(api.listDrives).mockResolvedValue([{ root: "D:\\", label: "", kind: "fixed", total: 0, free: 0, ready: true } as never]);
  await useTabViews.getState().load("t1", "drives:");
  expect(view().rows.map((r) => r.name)).toEqual(["D:\\"]);
  await dragOut("t1", "D:\\");
  vi.mocked(api.listDir).mockResolvedValueOnce([f("a.txt")]);
  await useTabViews.getState().load("t1", "C:\\x");
  await dragOut("t1", "..");
  expect(api.startDrag).not.toHaveBeenCalled();
});

test("programs open in the viewer with F3 and run with cm_Return", async () => {
  vi.mocked(api.listDir).mockResolvedValueOnce([f("setup.BAT")]);
  await useTabViews.getState().load("t1", "C:\\y");
  vi.mocked(api.openPath).mockResolvedValue();
  setCursor("setup.BAT");
  await executeCommand("cm_List");
  expect(activeTab(useLayout.getState()).file).toEqual({ path: "C:\\y\\setup.BAT", mode: "viewer" });
  expect(api.openPath).not.toHaveBeenCalled();
  useLayout.getState().activateTab("t1");
  await executeCommand("cm_Return");
  expect(api.openPath).toHaveBeenCalledWith("C:\\y\\setup.BAT");
});

test("suppressKeyboardContextMenu prevents keyboard menus outside text fields only", () => {
  const ev = (button: number, editable: boolean) => {
    const e = { button, target: { closest: () => (editable ? {} : null) }, preventDefault: vi.fn() };
    suppressKeyboardContextMenu(e as never);
    return e.preventDefault.mock.calls.length > 0;
  };
  expect(ev(-1, false)).toBe(true);
  expect(ev(0, false)).toBe(true);
  expect(ev(2, false)).toBe(false); // tasto destro: lo gestisce la lista (o resta il menu di WebView2)
  expect(ev(-1, true)).toBe(false); // menu di modifica del testo
  const noTarget = { button: -1, target: null, preventDefault: vi.fn() };
  suppressKeyboardContextMenu(noTarget as never);
  expect(noTarget.preventDefault).toHaveBeenCalled();
});

test("every shell command has a title in both languages", async () => {
  const en = (await import("../i18n/en.json")).default as { commands: Record<string, string> };
  const it = (await import("../i18n/it.json")).default as { commands: Record<string, string> };
  for (const id of SHELL_COMMAND_IDS) {
    expect(en.commands[id], id).toBeTruthy();
    expect(it.commands[id], id).toBeTruthy();
  }
});
