import { expect, test } from "vitest";
import { fileKind, formatAttrs, splitName } from "./format";

test("formatAttrs uses TC letters r a h s", () => {
  expect(formatAttrs(0)).toBe("----");
  expect(formatAttrs(0x1 | 0x20)).toBe("ra--");
  expect(formatAttrs(0x2 | 0x4)).toBe("--hs");
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
