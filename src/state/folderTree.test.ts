import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { childDirs, useFolderTree } from "./folderTree";

const listDir = vi.mocked(api.listDir);
const e = (name: string, isDir: boolean, attributes = 0): Entry => ({ name, isDir, size: 0, modifiedMs: 0, attributes, isSymlink: false });
const tree = () => useFolderTree.getState();

beforeEach(() => {
  vi.resetAllMocks();
  useFolderTree.setState({ expanded: {}, nodes: {} });
});

test("expanding lists the folder once and keeps only the subfolders", async () => {
  listDir.mockResolvedValueOnce([e("a.txt", false), e("b", true), e("a", true)]);
  await tree().toggle("C:\\x");
  expect(tree().expanded).toEqual({ "C:\\x": true });
  expect(tree().nodes["C:\\x"].status).toBe("ready");
  expect(tree().nodes["C:\\x"].dirs.map((d) => d.name)).toEqual(["b", "a"]);
  await tree().toggle("C:\\x"); // chiude senza rileggere
  expect(tree().expanded).toEqual({});
  expect(listDir).toHaveBeenCalledTimes(1);
});

test("childDirs sorts like the file list and respects hidden/system", () => {
  const dirs = [e("b10", true), e("b2", true), e(".git", true, 0x2), e("sys", true, 0x4), e("A", true)];
  expect(childDirs(dirs, false, false)).toEqual(["A", "b2", "b10"]);
  expect(childDirs(dirs, true, false)).toEqual([".git", "A", "b2", "b10"]);
  expect(childDirs(dirs, true, true)).toEqual([".git", "A", "b2", "b10", "sys"]);
});

test("a failing expand leaves the node collapsed with the error code; retry forgets it", async () => {
  listDir.mockRejectedValueOnce({ code: "ACCESS_DENIED", path: "C:\\x", detail: null });
  await tree().toggle("C:\\x");
  expect(tree().expanded).toEqual({});
  expect(tree().nodes["C:\\x"]).toMatchObject({ status: "error", error: "ACCESS_DENIED" });
  tree().retry("C:\\x");
  expect(tree().nodes["C:\\x"]).toBeUndefined();
});
