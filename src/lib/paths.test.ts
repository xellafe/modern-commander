import { expect, test } from "vitest";
import { parentPath } from "../state/tabView";
import {
  DRIVES, connectionPath, dirTarget, folderName, formatArchive, isArchive, isArchiveName, isRemote, isVirtual, isZipName, joinPath,
  normalizePath, parseArchive, parseRemote, pathCrumbs, rootPath, sourceCaps,
} from "./paths";

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

test("archive locations roundtrip like Loc", () => {
  const a = formatArchive("sftp://w/rel/100%/app.zip", "/a");
  expect(a).toBe("arc://sftp:%2F%2Fw%2Frel%2F100%25%2Fapp.zip/a");
  expect(parseArchive(a)).toEqual({ container: "sftp://w/rel/100%/app.zip", inner: "/a" });
  expect(formatArchive("C:\\rel\\app.zip")).toBe("arc://C:\\rel\\app.zip/");
  expect(parseArchive("arc://C:\\rel\\app.zip/conf")).toEqual({ container: "C:\\rel\\app.zip", inner: "/conf" });
  expect(parseArchive("ARC://C:\\a.zip")).toEqual({ container: "C:\\a.zip", inner: "/" }); // schema senza distinzione di maiuscole, formato sempre minuscolo
  expect(parseArchive("arc://C:\\a.zip/x/./y/../z/")?.inner).toBe("/x/z");
  expect(formatArchive("C:\\a.zip", "/x/../y/")).toBe("arc://C:\\a.zip/y");
  expect(parseArchive("arc://sftp:%2f%2fw%2fa.zip/")?.container).toBe("sftp://w/a.zip"); // %2f minuscolo accettato
  expect(parseArchive("C:\\a.zip")).toBeNull();
  expect(parseArchive("sftp://w/a.zip")).toBeNull();
  // Come `Loc::parse`: `%` fuori da %25 e %2F, contenitore senza nome di file, archivio dentro un archivio, schema ignoto.
  for (const bad of ["arc://C:\\a%20b.zip/", "arc://C:\\a%2", "arc://sftp:%2F%2Fw%2F/", "arc://C:\\/", "arc://arc:%2F%2FC:\\a.zip%2Fx/", "arc://smb:%2F%2Fw%2Fa.zip/"]) {
    expect(parseArchive(bad), bad).toBeNull();
  }
  expect(isArchive("arc://C:\\a.zip/")).toBe(true);
  expect(isArchive("C:\\a.zip")).toBe(false);
  expect(isVirtual("arc://C:\\a.zip/")).toBe(true);
  expect(isVirtual("sftp://w/")).toBe(true);
  expect(isVirtual("C:\\")).toBe(false);
  expect(connectionPath("arc://sftp:%2F%2Fw%2Fa.zip/d")).toBe("sftp://w/a.zip");
  expect(connectionPath("sftp://w/x")).toBe("sftp://w/x");
});

test("archive names are the ones previewKind knows; zip is the only writable one", () => {
  for (const n of ["a.zip", "A.ZIP", "a.7z", "a.tar", "a.tgz", "a.tar.gz"]) expect(isArchiveName(n), n).toBe(true);
  for (const n of ["a.gz", "a.txt", "zip"]) expect(isArchiveName(n), n).toBe(false);
  expect(isZipName("a.ZIP")).toBe(true);
  expect(isZipName("a.7z")).toBe(false);
});

test("archive caps: only a ZIP on a writable container can be written; no shell, hash or watcher", () => {
  expect(sourceCaps("arc://C:\\a.zip/").write).toBe(true);
  expect(sourceCaps("arc://sftp:%2F%2Fw%2Fa.zip/d").write).toBe(true);
  expect(sourceCaps("arc://C:\\a.7z/").write).toBe(false);
  expect(sourceCaps("arc://http:%2F%2Fh%2Fa.zip/").write).toBe(false);
  expect(sourceCaps("arc://C:\\a.zip/")).toEqual({ write: true, watch: false, shell: false, thumbs: false, hash: false, scan: false, dirSize: false });
});

test("parent of archive root is the container folder; inside it climbs by POSIX segments", () => {
  expect(parentPath("arc://C:\\r\\a.zip/")).toBe("C:\\r");
  expect(parentPath("arc://C:\\a.zip/")).toBe("C:\\");
  expect(parentPath("arc://sftp:%2F%2Fw%2Frel%2Fa.zip/")).toBe("sftp://w/rel");
  expect(folderName("arc://C:\\r\\a.zip/")).toBe("a.zip");
  expect(parentPath("arc://C:\\r\\a.zip/d")).toBe("arc://C:\\r\\a.zip/");
  expect(parentPath("arc://C:\\r\\a.zip/d/e")).toBe("arc://C:\\r\\a.zip/d");
  expect(folderName("arc://C:\\r\\a.zip/d/e")).toBe("e");
  expect(rootPath("arc://C:\\r\\a.zip/d/e")).toBe("arc://C:\\r\\a.zip/");
  expect(joinPath("arc://C:\\r\\a.zip/", "x")).toBe("arc://C:\\r\\a.zip/x");
  expect(joinPath("arc://C:\\r\\a.zip/d", "x")).toBe("arc://C:\\r\\a.zip/d/x");
  expect(normalizePath(" ARC://C:\\r\\a.zip/d/./ ")).toBe("arc://C:\\r\\a.zip/d");
});

test("crumbs through the archive", () => {
  const crumbs = pathCrumbs("arc://C:\\r\\a.zip/d");
  expect(crumbs.map((c) => c.label)).toEqual(["C:", "r", "a.zip", "d"]);
  expect(crumbs[2].path).toBe("arc://C:\\r\\a.zip/");
  expect(crumbs[3].path).toBe("arc://C:\\r\\a.zip/d");
  expect(crumbs[1].path).toBe("C:\\r");
  // Contenitore remoto: il primo crumb resta la connessione.
  expect(pathCrumbs("arc://sftp:%2F%2Fw%2Frel%2Fa.zip/x/y").map((c) => [c.label, c.path])).toEqual([
    ["w", "sftp://w/"], ["rel", "sftp://w/rel"], ["a.zip", "arc://sftp:%2F%2Fw%2Frel%2Fa.zip/"],
    ["x", "arc://sftp:%2F%2Fw%2Frel%2Fa.zip/x"], ["y", "arc://sftp:%2F%2Fw%2Frel%2Fa.zip/x/y"],
  ]);
});

test("dirTarget: Enter on an archive file opens it, but never inside an archive or from the drive list", () => {
  expect(dirTarget("C:\\r", { name: "a.zip", isDir: false })).toBe("arc://C:\\r\\a.zip/");
  expect(dirTarget("sftp://w/r", { name: "a.7z", isDir: false })).toBe("arc://sftp:%2F%2Fw%2Fr%2Fa.7z/");
  expect(dirTarget("C:\\r", { name: "a.txt", isDir: false })).toBeNull();
  expect(dirTarget("arc://C:\\r\\a.zip/", { name: "b.zip", isDir: false })).toBeNull();
  expect(dirTarget("arc://C:\\r\\a.zip/", { name: "d", isDir: true })).toBe("arc://C:\\r\\a.zip/d");
  expect(dirTarget("arc://C:\\r\\a.zip/", { name: "..", isDir: true, isParent: true })).toBe("C:\\r");
  expect(dirTarget(DRIVES, { name: "a.zip", isDir: false })).toBeNull();
});
