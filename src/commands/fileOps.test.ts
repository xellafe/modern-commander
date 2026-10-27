import { expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import { activateGroup, initialLayout } from "../state/layout";
import type { DialogRequest } from "../state/ui";
import { copyTargets, mkdirTarget, mustChangeDest, nameError, resolveDest, startCopyMove } from "./fileOps";

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
