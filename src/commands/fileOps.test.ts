import { expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import { activateGroup, initialLayout, useLayout } from "../state/layout";
import { useSettings } from "../state/settings";
import { useTabViews } from "../state/tabViews";
import { useUi, type DialogRequest } from "../state/ui";
import { copyTargets, mkdirTarget, mustChangeDest, nameError, registerFileOpCommands, resolveDest, startCopyMove } from "./fileOps";
import { executeCommand } from "./registry";

test("mustChangeDest blocks the source folder for a single group or a move, not for a copy", () => {
  const req = (move: boolean, requireChange: boolean) => ({ from: "C:\\a", move, requireChange });
  expect(mustChangeDest(req(false, false), "c:\\A")).toBe(false);
  expect(mustChangeDest(req(false, true), "c:\\A")).toBe(true);
  expect(mustChangeDest(req(true, false), "C:\\a")).toBe(true);
  expect(mustChangeDest(req(true, false), "C:\\b")).toBe(false);
});

test("copyTargets with one group proposes the current folder and requires a change", () => {
  const l = initialLayout(["C:\\a"]);
  expect(copyTargets(l)).toEqual({ dest: "C:\\a", requireChange: true, targets: [] });
});

test("copyTargets with two groups proposes the other group without a list", () => {
  const l = initialLayout(["C:\\a", "D:\\b"]);
  expect(copyTargets(l)).toEqual({ dest: "D:\\b", requireChange: false, targets: [] });
});

test("copyTargets with three or more groups lists the others numbered by position", () => {
  let l = initialLayout(["C:\\a", "D:\\b", "E:\\c"]);
  const [g1, g2, g3] = l.groups.map((g) => g.id);
  l = activateGroup(activateGroup(l, g3), g2); // attivo g2, il più recente prima di lui è g3
  const r = copyTargets(l);
  expect(r.dest).toBe("E:\\c");
  expect(r.requireChange).toBe(false);
  expect(r.targets).toEqual([
    { n: 1, groupId: g1, path: "C:\\a" },
    { n: 3, groupId: g3, path: "E:\\c" },
  ]);
});

test("nameError rejects empty names, Windows-invalid characters and trailing dots or spaces", () => {
  expect(nameError("ok.txt")).toBeNull();
  expect(nameError("  ")).toBe("ops.nameEmpty");
  for (const bad of ["a<b", "a>b", 'a"b', "a/b", "a\\b", "a|b", "a?b", "a*b", "a:b", "a.", "a "]) {
    expect(nameError(bad)).toBe("ops.nameInvalid");
  }
});

test("mkdirTarget resolves relative paths with intermediates and allows absolute paths", () => {
  expect(mkdirTarget("C:\\base", "nuova")).toEqual({ path: "C:\\base\\nuova", focus: "nuova" });
  expect(mkdirTarget("C:\\base", "a\\b\\c")).toEqual({ path: "C:\\base\\a\\b\\c", focus: "a" });
  expect(mkdirTarget("C:\\", "x")).toEqual({ path: "C:\\x", focus: "x" });
  expect(mkdirTarget("C:\\base", "D:\\altro\\x")).toEqual({ path: "D:\\altro\\x", focus: null });
  expect(mkdirTarget("C:\\base", "C:\\base\\y")).toEqual({ path: "C:\\base\\y", focus: "y" });
  expect(mkdirTarget("C:\\base", "\\\\srv\\share\\z")).toEqual({ path: "\\\\srv\\share\\z", focus: null });
  expect(mkdirTarget("C:\\base", "a\\b:c")).toEqual({ error: "ops.nameInvalid" });
  expect(mkdirTarget("C:\\base", "a/b")).toEqual({ error: "ops.nameInvalid" });
  expect(mkdirTarget("C:\\base", "")).toEqual({ error: "ops.nameEmpty" });
  expect(mkdirTarget("drives:", "x")).toEqual({ error: "ops.nameInvalid" });
});

test("mustChangeDest ignores a trailing backslash", () => {
  expect(mustChangeDest({ from: "C:\\a", move: true, requireChange: false }, "C:\\a\\")).toBe(true);
  expect(mustChangeDest({ from: "C:\\", move: true, requireChange: false }, "c:\\")).toBe(true);
});

test("resolveDest keeps absolute paths and resolves relative ones against the source folder", () => {
  // Il `\` finale resta: per il backend la destinazione è di sicuro una cartella (regola di TC per un file solo).
  expect(resolveDest("C:\\a", "D:/x/")).toBe("D:\\x\\");
  expect(resolveDest("C:\\a", "D:\\x")).toBe("D:\\x");
  expect(resolveDest("C:\\a", "sub")).toBe("C:\\a\\sub");
  expect(resolveDest("C:\\a", "sub\\\\")).toBe("C:\\a\\sub\\");
  expect(resolveDest("C:\\a", "\\\\srv\\share\\")).toBe("\\\\srv\\share\\");
  expect(resolveDest("C:\\a", "d:")).toBe("d:\\");
  expect(resolveDest("C:\\a", "  ")).toBeNull();
  expect(resolveDest("drives:", "sub")).toBeNull();
  expect(resolveDest("drives:", "drives:")).toBeNull();
});

test("remote names forbid only '/' and NUL; remote destinations resolve with '/'", () => {
  expect(nameError("a:b", "sftp://w/x")).toBeNull();
  expect(nameError("a:b", "C:\\x")).toBe("ops.nameInvalid");
  expect(nameError("trailing. ", "sftp://w/")).toBeNull();
  expect(nameError("a/b", "sftp://w/")).toBe("ops.nameInvalid");
  expect(nameError("a\0b", "sftp://w/")).toBe("ops.nameInvalid");
  expect(nameError("..", "sftp://w/")).toBe("ops.nameInvalid"); // risolto dal backend: rinominerebbe nella cartella sopra
  expect(nameError(" ", "sftp://w/")).toBe("ops.nameEmpty");

  expect(resolveDest("sftp://w/var", "log")).toBe("sftp://w/var/log");
  expect(resolveDest("sftp://w/var", "a/b/")).toBe("sftp://w/var/a/b/");
  expect(resolveDest("sftp://w/var", "../etc")).toBe("sftp://w/etc");
  expect(resolveDest("C:\\a", "sftp://w/x/")).toBe("sftp://w/x/");
  expect(resolveDest("sftp://w/var", "D:\\x")).toBe("D:\\x");
  expect(resolveDest("sftp://w/var", "sftp://w")).toBe("sftp://w/");
  expect(resolveDest("https://h/a", "sub")).toBeNull(); // relativa a una sorgente in sola lettura

  expect(mkdirTarget("sftp://w/var", "a:b/c")).toEqual({ path: "sftp://w/var/a:b/c", focus: "a:b" });
  expect(mkdirTarget("C:\\base", "sftp://w/var/n")).toEqual({ path: "sftp://w/var/n", focus: null });
  expect(mkdirTarget("sftp://w/var", "sftp://w/var/n")).toEqual({ path: "sftp://w/var/n", focus: "n" });
  expect(mkdirTarget("sftp://w/var", "D:\\n")).toEqual({ path: "D:\\n", focus: null });
  expect(mkdirTarget("sftp://w/var", "a/../b")).toEqual({ error: "ops.nameInvalid" });
  expect(mkdirTarget("http://h/", "x")).toEqual({ error: "ops.nameInvalid" });

  // Un `/` iniziale su una scheda remota è la radice del server, non una sottocartella.
  expect(resolveDest("sftp://w/var", "/etc/x")).toBe("sftp://w/etc/x");
  expect(resolveDest("sftp://w/var", "/etc/")).toBe("sftp://w/etc/");
  expect(mkdirTarget("sftp://w/var", "/etc/x")).toEqual({ path: "sftp://w/etc/x", focus: null });
  expect(mkdirTarget("sftp://w/var", "/var/n")).toEqual({ path: "sftp://w/var/n", focus: "n" });
  expect(mkdirTarget("sftp://w/var", "/VAR/n")).toEqual({ path: "sftp://w/VAR/n", focus: null }); // POSIX: maiuscole diverse

  expect(mustChangeDest({ from: "sftp://w/a", move: true, requireChange: false }, "sftp://w/a/")).toBe(true);
  expect(mustChangeDest({ from: "sftp://w/a", move: true, requireChange: false }, "sftp://w/A")).toBe(false);
});

test("archive locations behave like remote ones for names, destinations and new folders", () => {
  const z = "arc://C:\\r\\a.zip/conf";
  expect(nameError("a:b", z)).toBeNull();
  expect(nameError("a/b", z)).toBe("ops.nameInvalid");
  expect(nameError("..", z)).toBe("ops.nameInvalid");
  expect(resolveDest("C:\\r", z)).toBe(z); // una destinazione digitata in una scheda ZIP
  expect(resolveDest(z, "sub")).toBe(`${z}/sub`);
  expect(resolveDest(z, "/x/")).toBe("arc://C:\\r\\a.zip/x/");
  expect(resolveDest("arc://C:\\r\\a.7z/", "sub")).toBeNull(); // relativa a un archivio in sola lettura
  expect(mkdirTarget(z, "a:b/c")).toEqual({ path: `${z}/a:b/c`, focus: "a:b" });
  expect(mkdirTarget(z, "/new")).toEqual({ path: "arc://C:\\r\\a.zip/new", focus: null });
  expect(mkdirTarget(z, "/conf/n")).toEqual({ path: `${z}/n`, focus: "n" });
  expect(mkdirTarget("arc://C:\\r\\a.7z/", "x")).toEqual({ error: "ops.nameInvalid" });
  expect(mustChangeDest({ from: z, move: true, requireChange: false }, `${z}/`)).toBe(true);
});

// Comandi su una scheda sorgente (g1, `C:\r`, cursore su x.txt) con un'altra scheda a destra.
async function setup(paths: [string, string], confirmDelete = true) {
  vi.mocked(api.watchDir).mockResolvedValue();
  vi.mocked(api.unwatchDir).mockResolvedValue();
  vi.mocked(api.enqueueJob).mockClear();
  const l = initialLayout(paths);
  useLayout.setState(l);
  useTabViews.setState({ views: {}, notice: null });
  useUi.setState({ dialog: null });
  useSettings.setState({ confirmDelete });
  const [g1] = l.groups;
  const t1 = g1.activeTabId;
  vi.mocked(api.listDir).mockResolvedValueOnce([
    { name: "x.txt", isDir: false, size: 1, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false },
  ]);
  await useTabViews.getState().load(t1, paths[0]);
  useTabViews.getState().update(t1, (v) => ({ ...v, cursor: 1 }));
}

test("copy into a zip tab proposes the archive location; into a 7z, nothing (not writable)", async () => {
  registerFileOpCommands();
  await setup(["C:\\r", "arc://C:\\r\\a.zip/conf"]);
  await executeCommand("cm_Copy");
  expect(useUi.getState().dialog).toMatchObject({ kind: "copyMove", sources: ["C:\\r\\x.txt"], dest: "arc://C:\\r\\a.zip/conf" });
  await setup(["C:\\r", "arc://C:\\r\\a.7z/"]);
  await executeCommand("cm_Copy");
  expect(useUi.getState().dialog).toMatchObject({ kind: "copyMove", dest: "" });
  // Dalla scheda 7z, spostare o eliminare non si può: avviso, niente coda.
  await setup(["arc://C:\\r\\a.7z/", "C:\\r"]);
  await executeCommand("cm_RenMov");
  await executeCommand("cm_Delete");
  expect(useUi.getState().dialog).toBeNull();
  expect(useTabViews.getState().notice?.code).toBe("READ_ONLY_SOURCE");
});

test("inside an archive deleting is always permanent, with or without confirmation", async () => {
  registerFileOpCommands();
  const z = "arc://C:\\r\\a.zip/";
  await setup([z, "C:\\r"], false);
  await executeCommand("cm_Delete");
  expect(api.enqueueJob).toHaveBeenCalledOnce();
  expect(vi.mocked(api.enqueueJob).mock.lastCall?.[0]).toMatchObject({ kind: { type: "delete", permanent: true }, sources: [`${z}x.txt`] });
  await setup([z, "C:\\r"], true);
  await executeCommand("cm_Delete");
  expect(useUi.getState().dialog).toEqual({ kind: "delete", permanent: true, sources: [`${z}x.txt`] });
  // Su un disco locale F8 resta nel Cestino.
  await setup(["C:\\r", z], true);
  await executeCommand("cm_Delete");
  expect(useUi.getState().dialog).toMatchObject({ kind: "delete", permanent: false });
});

test("a single file copied to a remote 'dir/' keeps its name: the backend drops the trailing '/'", async () => {
  vi.mocked(api.enqueueJob).mockResolvedValue(1);
  vi.mocked(api.fileInfo).mockResolvedValue({ isDir: false } as never);
  const req: Extract<DialogRequest, { kind: "copyMove" }> = {
    kind: "copyMove", move: false, sources: ["C:\\a\\f.txt"], from: "C:\\a", dest: "", requireChange: false, targets: [], policy: "ask",
  };
  await startCopyMove(req, "sftp://w/new/", "", "ask");
  expect(vi.mocked(api.enqueueJob).mock.lastCall?.[0].dest).toBe("sftp://w/new/f.txt");
  // Una cartella va comunque dentro la destinazione: il nome non serve.
  vi.mocked(api.fileInfo).mockResolvedValue({ isDir: true } as never);
  await startCopyMove({ ...req, sources: ["C:\\a\\d"] }, "sftp://w/new/", "", "ask");
  expect(vi.mocked(api.enqueueJob).mock.lastCall?.[0].dest).toBe("sftp://w/new/");
});
