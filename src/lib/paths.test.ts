import { expect, test } from "vitest";
import { parentPath } from "../state/tabView";
import { DRIVES, dirTarget, folderName, isRemote, joinPath, normalizePath, parseRemote, pathCrumbs, rootPath, sourceCaps } from "./paths";

test("parseRemote follows the rules of Loc: scheme, connection id, POSIX path", () => {
  expect(parseRemote("sftp://w/var/log")).toEqual({ scheme: "sftp", conn: "w", path: "/var/log" });
  expect(parseRemote("SFTP://w")).toEqual({ scheme: "sftp", conn: "w", path: "/" });
  expect(parseRemote("ftp://my_srv-1//a/./b/../c/")).toEqual({ scheme: "ftp", conn: "my_srv-1", path: "/a/c" });
  expect(parseRemote("https://h/../..")).toEqual({ scheme: "https", conn: "h", path: "/" });
  expect(parseRemote("sftp://w/a%20b\\c")?.path).toBe("/a%20b\\c"); // letterale, niente decodifica
  for (const local of ["C:\\a", "C://x", "\\\\srv\\share", DRIVES, "sftp://", "sftp://bad.host/x", "smb://w/x", `sftp://${"a".repeat(65)}/x`]) {
    expect(parseRemote(local)).toBeNull();
  }
  expect(isRemote("ftps://w/x")).toBe(true);
  expect(isRemote("C:\\x")).toBe(false);
});

test("remote paths join, climb and split with '/'; the root has no parent", () => {
  expect(joinPath("sftp://w/", "a")).toBe("sftp://w/a");
  expect(joinPath("sftp://w/var", "log")).toBe("sftp://w/var/log");
  expect(parentPath("sftp://w/var/log")).toBe("sftp://w/var");
  expect(parentPath("sftp://w/var")).toBe("sftp://w/");
  expect(parentPath("sftp://w/")).toBeNull();
  expect(parentPath("sftp://w")).toBeNull();
  expect(rootPath("sftp://w/var/log")).toBe("sftp://w/");
  expect(pathCrumbs("sftp://w/var/a\\b")).toEqual([
    { label: "w", path: "sftp://w/" },
    { label: "var", path: "sftp://w/var" },
    { label: "a\\b", path: "sftp://w/var/a\\b" },
  ]);
  expect(folderName("http://site/")).toBe("site");
  expect(dirTarget("sftp://w/a", { name: "sub", isDir: true })).toBe("sftp://w/a/sub");
  expect(dirTarget("sftp://w/a", { name: "..", isDir: true, isParent: true })).toBe("sftp://w/");
});

test("normalizePath leaves '/' alone on an address with a scheme", () => {
  expect(normalizePath("sftp://w/var/log/")).toBe("sftp://w/var/log");
  expect(normalizePath(" SFTP://w//var/./x/.. ")).toBe("sftp://w/var");
  expect(normalizePath("sftp://w")).toBe("sftp://w/");
  // Un host vero (lo risolve il gestore delle connessioni) resta com'è scritto.
  expect(normalizePath("sftp://user@host.example:22/var/")).toBe("sftp://user@host.example:22/var/");
});

test("sourceCaps: everything locally, nothing in the drive list or over HTTP, only writing over SFTP and FTP", () => {
  const all = (v: boolean) => ({ write: v, watch: v, shell: v, thumbs: v, hash: v, scan: v, dirSize: v });
  expect(sourceCaps("C:\\a")).toEqual(all(true));
  expect(sourceCaps("\\\\srv\\share")).toEqual(all(true));
  expect(sourceCaps(DRIVES)).toEqual(all(false));
  for (const p of ["sftp://w/a", "ftp://w/", "ftps://w/x"]) expect(sourceCaps(p)).toEqual({ ...all(false), write: true });
  for (const p of ["http://w/a", "https://w/"]) expect(sourceCaps(p)).toEqual(all(false));
});

test("joinPath handles roots and UNC", () => {
  expect(joinPath("C:\\", "a")).toBe("C:\\a");
  expect(joinPath("C:\\a", "b")).toBe("C:\\a\\b");
  expect(joinPath("\\\\srv\\share", "x")).toBe("\\\\srv\\share\\x");
});

test("rootPath climbs to the drive or the UNC share", () => {
  expect(rootPath("C:\\a\\b")).toBe("C:\\");
  expect(rootPath("C:\\")).toBe("C:\\");
  expect(rootPath("\\\\srv\\share\\a\\b")).toBe("\\\\srv\\share");
  expect(rootPath("\\\\srv\\share")).toBe("\\\\srv\\share");
  expect(rootPath("\\\\srv")).toBe("\\\\srv");
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
