import { expect, test } from "vitest";
import { fileKind, formatAttrs, formatEta, formatMode, formatSize, splitName } from "./format";

test("formatSize uses B/KB/MB/GB with the language's decimal separator", () => {
  expect(formatSize(0, "it")).toBe("0 B");
  expect(formatSize(1536, "it")).toBe("1,5 KB");
  expect(formatSize(1536, "en")).toBe("1.5 KB");
  expect(formatSize(2.4 * 1024 ** 3, "it")).toBe("2,4 GB");
});

test("formatEta is m:ss under an hour, h:mm:ss above", () => {
  expect(formatEta(65)).toBe("1:05");
  expect(formatEta(3725)).toBe("1:02:05");
});

test("formatAttrs uses TC letters r a h s", () => {
  expect(formatAttrs(0)).toBe("----");
  expect(formatAttrs(0x1 | 0x20)).toBe("ra--");
  expect(formatAttrs(0x2 | 0x4)).toBe("--hs");
});

test("formatMode shows POSIX permissions like ls", () => {
  expect(formatMode(0o755, true)).toBe("drwxr-xr-x");
  expect(formatMode(0o644, false)).toBe("-rw-r--r--");
  expect(formatMode(0o100600, false)).toBe("-rw-------"); // il tipo nei bit alti non conta
});

test("splitName separates the extension of files only", () => {
  expect(splitName("a.tar.gz", false)).toEqual(["a.tar", "gz"]);
  expect(splitName(".gitignore", false)).toEqual([".gitignore", ""]);
  expect(splitName("noext", false)).toEqual(["noext", ""]);
  expect(splitName("my.folder", true)).toEqual(["my.folder", ""]);
});

test("fileKind groups extensions case-insensitively", () => {
  expect(fileKind("x", true)).toBe("folder");
  expect(fileKind("a.PNG", false)).toBe("image");
  expect(fileKind("main.tsx", false)).toBe("code");
  expect(fileKind("package.json", false)).toBe("data");
  expect(fileKind("a.zip", false)).toBe("archive");
  expect(fileKind("README", false)).toBe("file");
});
