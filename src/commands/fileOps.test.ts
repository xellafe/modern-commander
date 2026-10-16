import { expect, test } from "vitest";
import { activateGroup, initialLayout } from "../state/layout";
import { copyTargets, mkdirTarget, mustChangeDest, nameError, resolveDest } from "./fileOps";

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
