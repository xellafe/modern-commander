import { beforeAll, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { useLayout } from "../state/layout";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { NAVIGATION_COMMAND_IDS, nextSort, registerNavigationCommands } from "./navigation";
import { executeCommand } from "./registry";

const listDir = vi.mocked(api.listDir);
const f = (name: string, isDir = false): Entry => ({ name, isDir, size: 0, modifiedMs: 0, attributes: 0, isSymlink: false });
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
