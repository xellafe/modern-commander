import { beforeAll, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { useLayout } from "../state/layout";
import { useSettings } from "../state/settings";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { useUi } from "../state/ui";
import { dropOnGroup } from "./fileOps";
import { registerNavigationCommands } from "./navigation";
import { executeCommand } from "./registry";
import { dragOut, pastDragThreshold, registerShellCommands, rowsAt, SHELL_COMMAND_IDS, shellPaths, toPhysical } from "./shell";

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

test("cm_List opens the cursor file, cm_Edit uses the external editor; folders are ignored", async () => {
  vi.mocked(api.openPath).mockResolvedValue();
  vi.mocked(api.openWith).mockResolvedValue();
  useSettings.setState({ externalEditor: "code.exe" });
  setCursor("a.txt");
  await executeCommand("cm_List");
  await executeCommand("cm_Edit");
  expect(api.openPath).toHaveBeenCalledWith("C:\\x\\a.txt");
  expect(api.openWith).toHaveBeenCalledWith("code.exe", "C:\\x\\a.txt");
  setCursor("sub");
  await executeCommand("cm_List");
  await executeCommand("cm_Edit");
  expect(api.openPath).toHaveBeenCalledTimes(1);
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
  await executeCommand("cm_List");
  expect(useTabViews.getState().notice).toBeNull();
  vi.mocked(api.openPath).mockRejectedValueOnce({ code: "ACCESS_DENIED", path: "C:\\x\\a.txt", detail: null });
  await executeCommand("cm_List");
  expect(useTabViews.getState().notice).toEqual({ tabId: "t1", code: "ACCESS_DENIED", path: "C:\\x\\a.txt" });
});

test("drops are ignored while our own drag is running", async () => {
  let finish!: () => void;
  vi.mocked(api.startDrag).mockReturnValue(new Promise<void>((r) => (finish = r)));
  vi.mocked(api.normalizePaths).mockImplementation(async (p) => p);
  mark("a.txt", "b.txt");
  const drag = dragOut("t1", indexOf("b.txt"));
  expect(api.startDrag).toHaveBeenCalledWith(["C:\\x\\a.txt", "C:\\x\\b.txt"]);
  await dropOnGroup("g1", ["C:\\x\\a.txt"]);
  expect(useUi.getState().dialog).toBeNull();
  finish();
  await drag;
  await dropOnGroup("g1", ["C:\\y\\z.txt"]);
  expect(useUi.getState().dialog?.kind).toBe("copyMove");
});

test("every shell command has a title in both languages", async () => {
  const en = (await import("../i18n/en.json")).default as { commands: Record<string, string> };
  const it = (await import("../i18n/it.json")).default as { commands: Record<string, string> };
  for (const id of SHELL_COMMAND_IDS) {
    expect(en.commands[id], id).toBeTruthy();
    expect(it.commands[id], id).toBeTruthy();
  }
});
