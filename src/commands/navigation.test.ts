import { beforeAll, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
// La finestra di Tauri non esiste in Node: serve solo a catturare il gestore di chiusura.
const closeHandlers = vi.hoisted(() => [] as ((e: { preventDefault(): void }) => void)[]);
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onCloseRequested: async (h: (typeof closeHandlers)[number]) => void closeHandlers.push(h) }),
}));
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { activeTab, openFileTab, useLayout } from "../state/layout";
import { useEditors } from "../state/editors";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { useUi } from "../state/ui";
import { useViewers } from "../state/viewers";
import { closeTab, closeTabNow, cursorStep, ensureDirTab, initCloseGuard, NAVIGATION_COMMAND_IDS, nextSort, registerNavigationCommands, setGridColumns, setPageSize } from "./navigation";
import { executeCommand, getCommand } from "./registry";

const listDir = vi.mocked(api.listDir);
const f = (name: string, isDir = false): Entry => ({ name, isDir, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false });
const ctx = () => activeTabContext();

beforeAll(registerNavigationCommands);
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
  useEditors.setState({ docs: {} });
  listDir.mockResolvedValueOnce([f("sub", true), f("a.txt"), f("b.txt")]);
  await useTabViews.getState().load("t1", "C:\\x");
});

test("nextSort toggles desc only on the same key", () => {
  expect(nextSort({ key: "name", desc: false }, "name")).toEqual({ key: "name", desc: true });
  expect(nextSort({ key: "name", desc: true }, "size")).toEqual({ key: "size", desc: false });
});

test("cm_Reverse marks the cursor row and moves down; '..' is never marked", async () => {
  await executeCommand("cm_Reverse"); // su '..': nessuna marcatura, scende su 'sub'
  await executeCommand("mc_CursorDown"); // su 'a.txt'
  await executeCommand("cm_Reverse");
  expect([...ctx().view.marked]).toEqual(["a.txt"]);
  expect(ctx().cursorRow?.name).toBe("b.txt");
  expect(ctx().markedOrCursor.map((r) => r.name)).toEqual(["a.txt"]);
});

test("cm_Return enters a folder; cm_GoToParent comes back with the cursor on it", async () => {
  useTabViews.getState().update("t1", (v) => ({ ...v, cursor: 1 })); // sub
  listDir.mockResolvedValueOnce([]);
  await executeCommand("cm_Return");
  expect(ctx().path).toBe("C:\\x\\sub");
  listDir.mockResolvedValueOnce([f("sub", true), f("a.txt")]);
  await executeCommand("cm_GoToParent");
  expect(ctx().path).toBe("C:\\x");
  expect(ctx().cursorRow?.name).toBe("sub");
});

test("cm_Return on a remote file opens the internal viewer instead of the associated program", async () => {
  listDir.mockResolvedValueOnce([f("log.txt")]);
  await useTabViews.getState().load("t1", "sftp://w/var");
  useTabViews.getState().update("t1", (v) => ({ ...v, cursor: 1 })); // log.txt
  await executeCommand("cm_Return");
  expect(api.openPath).not.toHaveBeenCalled();
  expect(activeTab(useLayout.getState()).file).toEqual({ path: "sftp://w/var/log.txt", mode: "viewer" });
});

test("sort command on the same key twice reverses the order", async () => {
  await executeCommand("cm_SrcByName"); // già per nome: diventa decrescente
  expect(ctx().view.rows.map((r) => r.name)).toEqual(["..", "sub", "b.txt", "a.txt"]);
});

test("every navigation command has a title in both languages", async () => {
  const en = (await import("../i18n/en.json")).default as { commands: Record<string, string> };
  const it = (await import("../i18n/it.json")).default as { commands: Record<string, string> };
  for (const id of NAVIGATION_COMMAND_IDS) {
    expect(en.commands[id], id).toBeTruthy();
    expect(it.commands[id], id).toBeTruthy();
  }
});

test("cm_SrcQuickview is a group command (not inside fields or dialogs) and toggles the inspector", async () => {
  expect(getCommand("cm_SrcQuickview")?.when).toBe("group");
  useUi.setState({ inspectorVisible: false });
  await executeCommand("cm_SrcQuickview");
  expect(useUi.getState().inspectorVisible).toBe(true);
});

test("ensureDirTab returns the directory tab itself, the opener from a file tab, or opens a new directory tab", () => {
  expect(ensureDirTab()).toBe("t1");
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\f.txt", mode: "viewer" }));
  expect(ensureDirTab()).toBe("t1");
  expect(activeTab(useLayout.getState()).id).toBe("t1");
  // L'aprente è stato chiuso: nasce una scheda cartella sulla cartella del file.
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\g.txt", mode: "viewer" }));
  useLayout.setState({ tabs: { ...useLayout.getState().tabs, [activeTab(useLayout.getState()).id]: { ...activeTab(useLayout.getState()), returnTo: "gone" } } });
  const id = ensureDirTab();
  expect(activeTab(useLayout.getState())).toMatchObject({ id, path: "C:\\x" });
  expect(activeTab(useLayout.getState()).file).toBeUndefined();
});

test("closing a viewer tab frees its viewer state, closing the last tab of the last group does not", () => {
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\f.txt", mode: "viewer" }));
  const id = activeTab(useLayout.getState()).id;
  useViewers.setState({ states: { [id]: { mode: "text", encoding: "utf-8", wrap: false, top: 0, left: 0 }, t1: { mode: "hex", encoding: "utf-8", wrap: false, top: 0, left: 0 } } });
  closeTab(id);
  expect(Object.keys(useViewers.getState().states)).toEqual(["t1"]);
  closeTab("t1"); // l'ultima scheda dell'ultimo gruppo resta
  expect(useViewers.getState().states.t1).toBeDefined();
});

test("closing the only tab of the last group when it is a file tab turns it into a folder tab on its folder", () => {
  const file = { path: "C:\\x\\f.txt", mode: "viewer" as const };
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["f1"], activeTabId: "f1" }],
    tabs: { f1: { id: "f1", path: "C:\\x", file } },
    activeGroupId: "g1",
    activationOrder: ["g1"],
  });
  useViewers.setState({ states: { f1: { mode: "text", encoding: "utf-8", wrap: false, top: 0, left: 0 } } });
  closeTab("f1");
  const l = useLayout.getState();
  expect(l.tabs.f1).toBeUndefined();
  expect(l.groups).toHaveLength(1);
  expect(l.groups[0].tabIds).toHaveLength(1);
  expect(activeTab(l)).toMatchObject({ path: "C:\\x" });
  expect(activeTab(l).file).toBeUndefined();
  expect(useViewers.getState().states.f1).toBeUndefined();
});

const editorDoc = (dirty: boolean) => ({ path: "C:\\x\\f.txt", encoding: "utf-8", bom: false, modifiedMs: 1, dirty, readOnly: false });

test("closing a modified editor asks first and keeps the tab; closeTabNow closes it and frees the document", () => {
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\f.txt", mode: "editor" }));
  const id = activeTab(useLayout.getState()).id;
  useEditors.setState({ docs: { [id]: editorDoc(true) } });
  closeTab(id);
  expect(useUi.getState().dialog).toEqual({ kind: "unsaved", tabId: id });
  expect(useLayout.getState().tabs[id]).toBeDefined();
  expect(useEditors.getState().docs[id]).toBeDefined();
  closeTabNow(id);
  expect(useLayout.getState().tabs[id]).toBeUndefined();
  expect(useEditors.getState().docs[id]).toBeUndefined();
});

test("closing an unmodified editor closes it without asking", () => {
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\f.txt", mode: "editor" }));
  const id = activeTab(useLayout.getState()).id;
  useEditors.setState({ docs: { [id]: editorDoc(false) } });
  closeTab(id);
  expect(useUi.getState().dialog).toBeNull();
  expect(useLayout.getState().tabs[id]).toBeUndefined();
});

test("the window close guard also counts unsaved editors", async () => {
  await initCloseGuard();
  const guard = closeHandlers[closeHandlers.length - 1];
  const event = () => ({ preventDefault: vi.fn() });
  const clean = event();
  guard(clean);
  expect(clean.preventDefault).not.toHaveBeenCalled(); // niente job né modifiche: la finestra si chiude
  useEditors.setState({ docs: { a: editorDoc(true), b: editorDoc(false), c: editorDoc(true) } });
  const blocked = event();
  guard(blocked);
  expect(blocked.preventDefault).toHaveBeenCalled();
  expect(useUi.getState().dialog).toEqual({ kind: "exit", count: 0, dirty: 2 });
});

test("tab and group commands are 'group', so they stay available on file tabs", async () => {
  const { getCommand } = await import("./registry");
  for (const id of ["cm_OpenNewTab", "cm_CloseCurrentTab", "cm_SwitchToNextTab", "cm_SwitchToPreviousTab", "mc_FocusNextGroup", "mc_FocusPrevGroup", "mc_SplitGroup"]) {
    expect(getCommand(id)?.when).toBe("group");
  }
});

test("ensureDirTab(path) opens the fallback tab directly on the target, so Group's initial load and the caller agree", () => {
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\f.txt", mode: "viewer" }));
  useLayout.setState({ tabs: { ...useLayout.getState().tabs, [activeTab(useLayout.getState()).id]: { ...activeTab(useLayout.getState()), returnTo: "gone" } } });
  const id = ensureDirTab("C:\\target");
  expect(activeTab(useLayout.getState())).toMatchObject({ id, path: "C:\\target" });
  expect(activeTab(useLayout.getState()).file).toBeUndefined();
});

test("ensureDirTab does not go back to an opener that is itself a file tab", () => {
  const l = useLayout.getState();
  useLayout.setState(openFileTab(l, "g1", { path: "C:\\x\\a.txt", mode: "viewer" }));
  const a = activeTab(useLayout.getState()).id;
  useLayout.setState(openFileTab(useLayout.getState(), "g1", { path: "C:\\x\\b.txt", mode: "viewer" })); // returnTo = a (file)
  const id = ensureDirTab("C:\\target");
  expect(id).not.toBe(a);
  expect(activeTab(useLayout.getState())).toMatchObject({ id, path: "C:\\target" });
  expect(activeTab(useLayout.getState()).file).toBeUndefined();
});

const twoGroupsWithFileOnTheRight = () => {
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1"], activeTabId: "t1" }, { id: "g2", tabIds: ["t2"], activeTabId: "t2" }],
    tabs: { t1: { id: "t1", path: "C:\\x" }, t2: { id: "t2", path: "C:\\z", file: { path: "C:\\z\\f.txt", mode: "viewer" } } },
    activeGroupId: "g1",
    activationOrder: ["g1", "g2"],
  });
};

test("cm_Exchange does nothing when the other group shows a file tab", async () => {
  twoGroupsWithFileOnTheRight();
  const before = useLayout.getState().tabs;
  listDir.mockClear();
  await executeCommand("cm_Exchange");
  expect(listDir).not.toHaveBeenCalled();
  expect(useLayout.getState().tabs).toBe(before);
  expect(useTabViews.getState().views.t2).toBeUndefined();
});

test("mc_OpenInNextGroup opens a new folder tab when the target group shows a file tab, without loading into it", async () => {
  twoGroupsWithFileOnTheRight();
  await executeCommand("mc_CursorDown"); // da '..' a 'sub'
  listDir.mockClear();
  await executeCommand("mc_OpenInNextGroup");
  const l = useLayout.getState();
  expect(l.groups[1].tabIds).toHaveLength(2);
  expect(l.tabs[l.groups[1].activeTabId]).toMatchObject({ path: "C:\\x\\sub" });
  expect(l.tabs[l.groups[1].activeTabId].file).toBeUndefined();
  expect(l.tabs.t2.file).toBeDefined();
  expect(useTabViews.getState().views.t2).toBeUndefined();
  expect(l.activeGroupId).toBe("g1"); // come con un load: il gruppo attivo non cambia
});

test("cursorStep", () => {
  expect(cursorStep("thumbs", "down", 5, 4)).toBe(4);
  expect(cursorStep("thumbs", "up", 5, 4)).toBe(-4);
  expect(cursorStep("thumbs", "left", 5, 4)).toBe(-1);
  expect(cursorStep("thumbs", "right", 5, 4)).toBe(1);
  expect(cursorStep("thumbs", "pageDown", 5, 4)).toBe(20);
  expect(cursorStep("thumbs", "pageUp", 5, 4)).toBe(-20);
  expect(cursorStep("brief", "right", 5, 4)).toBe(5);
  expect(cursorStep("brief", "left", 5, 4)).toBe(-5);
  expect(cursorStep("brief", "down", 5, 4)).toBe(1);
  expect(cursorStep("brief", "pageDown", 5, 4)).toBe(4);
  expect(cursorStep("full", "right", 5, 4)).toBe(0);
  expect(cursorStep("full", "up", 5, 4)).toBe(-1);
  expect(cursorStep("full", "pageUp", 5, 4)).toBe(-4);
});

test("cm_SrcThumbs shows the grid; the arrows then move by tile and by row of tiles; Ctrl+F1/F2 views come back", async () => {
  useTabViews.setState({ views: { t1: { ...ctx().view, rows: Array.from({ length: 30 }, (_, i) => ({ ...ctx().view.rows[1], name: `f${i}` })) } } });
  setGridColumns(4);
  setPageSize(3);
  await executeCommand("cm_SrcThumbs");
  expect(ctx().view.view).toBe("thumbs");
  await executeCommand("mc_CursorDown");
  expect(ctx().view.cursor).toBe(4);
  await executeCommand("mc_CursorRight");
  expect(ctx().view.cursor).toBe(5);
  await executeCommand("mc_CursorPageDown");
  expect(ctx().view.cursor).toBe(17);
  await executeCommand("mc_CursorLeft");
  await executeCommand("mc_CursorUp");
  expect(ctx().view.cursor).toBe(12);
  await executeCommand("mc_MarkDown"); // marca e scende di una riga di riquadri
  expect([...ctx().view.marked]).toEqual(["f12"]);
  expect(ctx().view.cursor).toBe(16);
  await executeCommand("cm_SrcShort");
  expect(ctx().view.view).toBe("brief");
  await executeCommand("cm_SrcThumbs");
  await executeCommand("cm_SrcLong");
  expect(ctx().view.view).toBe("full");
});
