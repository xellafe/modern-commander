import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { useLayout } from "./layout";
import { useSettings } from "./settings";
import { useTabViews } from "./tabViews";

const listDir = vi.mocked(api.listDir);
const nearestExistingAncestor = vi.mocked(api.nearestExistingAncestor);
const f = (name: string): Entry => ({ name, isDir: false, size: 0, modifiedMs: 0, attributes: 0, isSymlink: false });

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(api.listDrives).mockResolvedValue([]);
  vi.mocked(api.watchDir).mockResolvedValue();
  vi.mocked(api.unwatchDir).mockResolvedValue();
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1"], activeTabId: "t1" }],
    tabs: { t1: { id: "t1", path: "C:\\" } },
    activeGroupId: "g1",
    activationOrder: ["g1"],
  });
  useTabViews.setState({ views: {}, notice: null });
});

test("missing folder falls back to nearest ancestor", async () => {
  listDir.mockRejectedValueOnce({ code: "NOT_FOUND" }).mockResolvedValueOnce([]);
  nearestExistingAncestor.mockResolvedValue("D:\\a");
  await useTabViews.getState().load("t1", "D:\\a\\gone");
  expect(useLayout.getState().tabs["t1"].path).toBe("D:\\a");
  expect(useTabViews.getState().notice?.code).toBe("NOT_FOUND");
});

test("no ancestor shows drives", async () => {
  listDir.mockRejectedValue({ code: "NOT_READY" });
  nearestExistingAncestor.mockResolvedValue(null);
  await useTabViews.getState().load("t1", "E:\\x");
  expect(useLayout.getState().tabs["t1"].path).toBe("drives:");
});

test("a first load failing with another error shows the drives instead of an empty tab", async () => {
  vi.mocked(api.listDrives).mockResolvedValue([{ root: "C:\\", label: "", kind: "fixed", total: 1, free: 1, ready: true }]);
  listDir.mockRejectedValueOnce({ code: "ACCESS_DENIED", path: "C:\\x", detail: null });
  await useTabViews.getState().load("t1", "C:\\x");
  expect(useLayout.getState().tabs["t1"].path).toBe("drives:");
  expect(useTabViews.getState().views["t1"].rows.map((r) => r.name)).toEqual(["C:\\"]);
  expect(useTabViews.getState().notice).toEqual({ tabId: "t1", code: "ACCESS_DENIED", path: "C:\\x" });
  // Una scheda che ha già una cartella ci resta: solo il notice.
  listDir.mockResolvedValueOnce([f("a")]);
  await useTabViews.getState().load("t1", "D:\\a");
  listDir.mockRejectedValueOnce({ code: "ACCESS_DENIED", path: "D:\\b", detail: null });
  await useTabViews.getState().load("t1", "D:\\b");
  expect(useLayout.getState().tabs["t1"].path).toBe("D:\\a");
  expect(useTabViews.getState().notice?.path).toBe("D:\\b");
});

test("reload keeps cursor on the same name", async () => {
  listDir.mockResolvedValueOnce([f("a"), f("b"), f("c")]);
  await useTabViews.getState().load("t1", "D:\\a");
  const v = useTabViews.getState().views["t1"];
  useTabViews.getState().update("t1", (x) => ({ ...x, cursor: x.rows.findIndex((r) => r.name === "b") }));
  expect(v.rows.map((r) => r.name)).toEqual(["..", "a", "b", "c"]);

  listDir.mockResolvedValueOnce([f("a"), f("aa"), f("b"), f("c")]);
  await useTabViews.getState().reload("t1");
  const after = useTabViews.getState().views["t1"];
  expect(after.rows.map((r) => r.name)).toEqual(["..", "a", "aa", "b", "c"]);
  expect(after.rows[after.cursor].name).toBe("b");
});

test("a stale listing does not overwrite a newer navigation", async () => {
  let resolveSlow!: (e: Entry[]) => void;
  listDir
    .mockReturnValueOnce(new Promise((r) => { resolveSlow = r; }))
    .mockResolvedValueOnce([f("new")]);
  const slow = useTabViews.getState().load("t1", "D:\\slow");
  await useTabViews.getState().load("t1", "D:\\fast");
  resolveSlow([f("old")]);
  await slow;
  expect(useLayout.getState().tabs["t1"].path).toBe("D:\\fast");
  expect(useTabViews.getState().views["t1"].rows.map((r) => r.name)).toEqual(["..", "new"]);
});

test("a change notified during a load is reread afterwards, without cancelling the load", async () => {
  listDir.mockResolvedValueOnce([f("a")]);
  await useTabViews.getState().load("t1", "D:\\a");
  let resolveNav!: (e: Entry[]) => void;
  listDir.mockReturnValueOnce(new Promise((r) => { resolveNav = r; })).mockResolvedValueOnce([f("x"), f("y")]);
  const nav = useTabViews.getState().load("t1", "D:\\b");
  await useTabViews.getState().reload("t1"); // arriva durante la navigazione
  resolveNav([f("x")]);
  await nav;
  await vi.waitFor(() => expect(useTabViews.getState().views["t1"].rows.map((r) => r.name)).toEqual(["..", "x", "y"]));
  expect(useLayout.getState().tabs["t1"].path).toBe("D:\\b");
  expect(listDir).toHaveBeenLastCalledWith("D:\\b");
});

// Le chiamate al watcher registrano il loro completamento; watchDir su "slow" è lento.
function recordWatcher() {
  const done: string[] = [];
  vi.mocked(api.watchDir).mockImplementation((_tab, path) =>
    new Promise((r) => setTimeout(() => { done.push(`watch ${path}`); r(); }, path.includes("slow") ? 40 : 0)));
  vi.mocked(api.unwatchDir).mockImplementation(async () => { done.push("unwatch"); });
  return done;
}

test("closing a tab right after navigating never leaks a watcher", async () => {
  const done = recordWatcher();
  listDir.mockResolvedValue([]);
  await useTabViews.getState().load("t1", "\\\\slow\\share");
  useTabViews.getState().dispose("t1");
  await vi.waitFor(() => expect(done).toEqual(["watch \\\\slow\\share", "unwatch"]));
});

test("a slow watch does not overwrite the watch of a later folder", async () => {
  const done = recordWatcher();
  listDir.mockResolvedValue([]);
  await useTabViews.getState().load("t1", "\\\\slow\\share");
  await useTabViews.getState().load("t1", "C:\\x");
  await vi.waitFor(() => expect(done).toEqual(["watch \\\\slow\\share", "watch C:\\x"]));
});

test("going up puts the cursor on the folder we came from", async () => {
  listDir.mockResolvedValueOnce([f("x"), { ...f("sub"), isDir: true }]);
  await useTabViews.getState().load("t1", "D:\\a", { focus: "sub" });
  const v = useTabViews.getState().views["t1"];
  expect(v.rows[v.cursor].name).toBe("sub");
});

test("rebuildAll applies showHidden to every open tab and keeps each cursor on the same name", async () => {
  const hidden = (name: string): Entry => ({ ...f(name), attributes: 0x2 });
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1", "t2"], activeTabId: "t1" }],
    tabs: { t1: { id: "t1", path: "C:\\" }, t2: { id: "t2", path: "C:\\" } },
  });
  listDir.mockResolvedValue([f("a"), hidden(".h"), f("b")]);
  await useTabViews.getState().load("t1", "D:\\a");
  await useTabViews.getState().load("t2", "D:\\a");
  const names = (id: string) => useTabViews.getState().views[id].rows.map((r) => r.name);
  expect(names("t1")).toEqual(["..", "a", "b"]);
  for (const id of ["t1", "t2"]) {
    useTabViews.getState().update(id, (x) => ({ ...x, cursor: x.rows.findIndex((r) => r.name === "b") }));
  }

  useSettings.setState({ showHidden: true });
  useTabViews.getState().rebuildAll();
  for (const id of ["t1", "t2"]) {
    const v = useTabViews.getState().views[id];
    expect(names(id)).toEqual(["..", ".h", "a", "b"]);
    expect(v.rows[v.cursor].name).toBe("b");
  }

  useSettings.setState({ showHidden: false });
  useTabViews.getState().rebuildAll();
  expect(names("t2")).toEqual(["..", "a", "b"]);
  expect(useTabViews.getState().views["t2"].rows[useTabViews.getState().views["t2"].cursor].name).toBe("b");
});
