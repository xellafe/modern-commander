import { expect, test } from "vitest";
import { DRIVES, dirTarget, joinPath, normalizePath, pathCrumbs, rootPath } from "./paths";

test("joinPath handles roots and UNC", () => {
  expect(joinPath("C:\\", "a")).toBe("C:\\a");
  expect(joinPath("C:\\a", "b")).toBe("C:\\a\\b");
  expect(joinPath("\\\\srv\\share", "x")).toBe("\\\\srv\\share\\x");
});

test("rootPath climbs to the drive or the UNC server", () => {
  expect(rootPath("C:\\a\\b")).toBe("C:\\");
  expect(rootPath("C:\\")).toBe("C:\\");
  expect(rootPath("\\\\srv\\share\\a\\b")).toBe("\\\\srv");
  expect(rootPath(DRIVES)).toBe(DRIVES);
});

test("pathCrumbs lists every ancestor, root first", () => {
  expect(pathCrumbs("D:\\Generale\\mc")).toEqual([
    { label: "D:", path: "D:\\" },
    { label: "Generale", path: "D:\\Generale" },
    { label: "mc", path: "D:\\Generale\\mc" },
  ]);
  expect(pathCrumbs("\\\\srv\\share\\x")).toEqual([
    { label: "\\\\srv", path: "\\\\srv" },
    { label: "share", path: "\\\\srv\\share" },
    { label: "x", path: "\\\\srv\\share\\x" },
  ]);
});

test("dirTarget: folder, '..', drive, file", () => {
  const dir = { name: "sub", isDir: true };
  expect(dirTarget("C:\\a", dir)).toBe("C:\\a\\sub");
  expect(dirTarget("C:\\a", { name: "..", isDir: true, isParent: true })).toBe("C:\\");
  expect(dirTarget(DRIVES, { name: "E:\\", isDir: true, drive: {} })).toBe("E:\\");
  expect(dirTarget("C:\\a", { name: "f.txt", isDir: false })).toBeNull();
});

test("normalizePath cleans a typed path", () => {
  expect(normalizePath(" c: ")).toBe("c:\\");
  expect(normalizePath("C:/Windows/")).toBe("C:\\Windows");
  expect(normalizePath("D:\\a\\\\")).toBe("D:\\a");
  expect(normalizePath("\\\\srv\\share\\")).toBe("\\\\srv\\share");
});
