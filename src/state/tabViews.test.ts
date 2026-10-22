import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { useLayout } from "./layout";
import { useRecentDirs } from "./recentDirs";
import { useSettings } from "./settings";
import { useTabViews } from "./tabViews";
import { useTypeNames } from "./typeNames";
import { DEFAULT_COLUMNS, toggleColumn } from "../lib/columns";

const listDir = vi.mocked(api.listDir);
const nearestExistingAncestor = vi.mocked(api.nearestExistingAncestor);
const f = (name: string): Entry => ({ name, isDir: false, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false });

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
  useTypeNames.setState({ names: {} });
  useSettings.setState({ columns: DEFAULT_COLUMNS, extInName: false });
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

test("una cartella caricata finisce tra le recenti; unità, riletture e cartelle sparite no", async () => {
  useRecentDirs.setState({ dirs: [] });
  listDir.mockResolvedValue([]);
  await useTabViews.getState().load("t1", "C:\\a");
  await useTabViews.getState().load("t1", "C:\\b");
  expect(useRecentDirs.getState().dirs).toEqual(["C:\\b", "C:\\a"]);
  await useTabViews.getState().load("t1", "C:\\a", { reload: true });
  await useTabViews.getState().load("t1", "drives:");
  expect(useRecentDirs.getState().dirs).toEqual(["C:\\b", "C:\\a"]);
  // cartella sparita: si registra quella dove si è finiti, mai quella richiesta
  listDir.mockRejectedValueOnce({ code: "NOT_FOUND" }).mockResolvedValueOnce([]);
  nearestExistingAncestor.mockResolvedValue("C:\\a");
  await useTabViews.getState().load("t1", "C:\\gone");
  expect(useRecentDirs.getState().dirs).toEqual(["C:\\a", "C:\\b"]);
});

const typeNames = vi.mocked(api.typeNames);
const rowNames = () => useTabViews.getState().views["t1"].rows.map((r) => r.name);

test("type names are not requested when the column is hidden and sort is not type", async () => {
  listDir.mockResolvedValue([f("b.txt"), f("a.png")]);
  await useTabViews.getState().load("t1", "C:\\x");
  await Promise.resolve();
  expect(typeNames).not.toHaveBeenCalled();
});

test("sort by type reorders once names arrive", async () => {
  listDir.mockResolvedValue([f("b.txt"), f("a.png")]);
  let resolve!: (v: string[]) => void;
  typeNames.mockReturnValueOnce(new Promise((r) => (resolve = r)));
  await useTabViews.getState().load("t1", "C:\\x");
  useTabViews.getState().update("t1", (v) => ({ ...v, cursor: v.rows.findIndex((r) => r.name === "a.png") }));
  useTabViews.getState().rebuild("t1", { sort: { key: "type", desc: false } });
  expect(rowNames()).toEqual(["..", "a.png", "b.txt"]); // testi non ancora arrivati: decide il nome
  expect(typeNames).toHaveBeenCalledWith(["txt", "png"]);
  resolve(["Documento di testo", "Immagine PNG"]);
  await vi.waitFor(() => expect(rowNames()).toEqual(["..", "b.txt", "a.png"]));
  const v = useTabViews.getState().views["t1"];
  expect(v.rows[v.cursor].name).toBe("a.png");
  expect(typeNames).toHaveBeenCalledTimes(1);
});

test("reload does not refetch known type names", async () => {
  useSettings.setState({ columns: toggleColumn(DEFAULT_COLUMNS, "type") });
  listDir.mockResolvedValue([f("b.txt")]);
  typeNames.mockResolvedValue(["Documento di testo"]);
  await useTabViews.getState().load("t1", "C:\\x");
  await vi.waitFor(() => expect(useTypeNames.getState().names).toEqual({ txt: "Documento di testo" }));
  await useTabViews.getState().reload("t1");
  await Promise.resolve();
  expect(typeNames).toHaveBeenCalledTimes(1);
});

test("two tabs sorted by type both reorder when shared names arrive", async () => {
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1"], activeTabId: "t1" }, { id: "g2", tabIds: ["t2"], activeTabId: "t2" }],
    tabs: { t1: { id: "t1", path: "C:\\x" }, t2: { id: "t2", path: "C:\\x" } },
    activeGroupId: "g1",
    activationOrder: ["g1", "g2"],
  });
  listDir.mockResolvedValue([f("b.txt"), f("a.png")]);
  let resolve!: (v: string[]) => void;
  typeNames.mockReturnValueOnce(new Promise((r) => (resolve = r)));
  await useTabViews.getState().load("t1", "C:\\x");
  await useTabViews.getState().load("t2", "C:\\x");
  useTabViews.getState().rebuild("t1", { sort: { key: "type", desc: false } });
  useTabViews.getState().rebuild("t2", { sort: { key: "type", desc: false } }); // le chiavi sono già in volo per t1
  resolve(["Documento di testo", "Immagine PNG"]);
  const names = (id: string) => useTabViews.getState().views[id].rows.map((r) => r.name);
  await vi.waitFor(() => {
    expect(names("t1")).toEqual(["..", "b.txt", "a.png"]);
    expect(names("t2")).toEqual(["..", "b.txt", "a.png"]);
  });
  expect(typeNames).toHaveBeenCalledTimes(1);
});

test("a network error on a remote tab leaves it where it is, with the notice and without nearestExistingAncestor", async () => {
  listDir.mockResolvedValueOnce([f("a")]);
  await useTabViews.getState().load("t1", "sftp://w/var");
  listDir.mockRejectedValueOnce({ code: "CONNECTION_FAILED", path: null, detail: null });
  await useTabViews.getState().reload("t1");
  listDir.mockRejectedValueOnce({ code: "NOT_FOUND", path: "sftp://w/var/gone", detail: null });
  await useTabViews.getState().load("t1", "sftp://w/var/gone");
  expect(useLayout.getState().tabs["t1"].path).toBe("sftp://w/var");
  expect(useTabViews.getState().views["t1"].rows.map((r) => r.name)).toEqual(["..", "a"]);
  expect(useTabViews.getState().notice?.code).toBe("NOT_FOUND");
  expect(nearestExistingAncestor).not.toHaveBeenCalled();
});

test("a remote tab failing its first load stays on its address, empty, so that Ctrl+R can retry", async () => {
  listDir.mockRejectedValueOnce({ code: "CONNECTION_FAILED", path: null, detail: null });
  await useTabViews.getState().load("t1", "sftp://w/var");
  expect(useLayout.getState().tabs["t1"].path).toBe("sftp://w/var");
  expect(useTabViews.getState().notice?.code).toBe("CONNECTION_FAILED");
  expect(api.listDrives).not.toHaveBeenCalled();
  listDir.mockResolvedValueOnce([f("a")]);
  await useTabViews.getState().reload("t1");
  expect(useTabViews.getState().views["t1"].rows.map((r) => r.name)).toEqual(["..", "a"]);
});

test("remote tabs are not watched", async () => {
  const done = recordWatcher();
  listDir.mockResolvedValue([]);
  await useTabViews.getState().load("t1", "C:\\x");
  await useTabViews.getState().load("t1", "sftp://w/");
  await vi.waitFor(() => expect(done).toEqual(["watch C:\\x", "unwatch"]));
});
