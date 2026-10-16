import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Drive } from "../ipc/types";
import { isUnwatched, useDrives } from "./drives";

const listDrives = vi.mocked(api.listDrives);
const drive = (root: string): Drive => ({ root, label: "", kind: "fixed", total: 10, free: 5, ready: true });

beforeEach(() => {
  vi.resetAllMocks();
  useDrives.setState({ drives: [] });
});

test("refresh never starts a second listDrives while one is pending", async () => {
  let finish!: (d: Drive[]) => void;
  listDrives.mockReturnValueOnce(new Promise((r) => (finish = r)));
  const first = useDrives.getState().refresh();
  await useDrives.getState().refresh(); // saltata
  await useDrives.getState().refresh(); // saltata
  expect(listDrives).toHaveBeenCalledTimes(1);
  finish([drive("C:\\")]);
  await first;
  expect(useDrives.getState().drives.map((d) => d.root)).toEqual(["C:\\"]);

  listDrives.mockResolvedValueOnce([drive("C:\\"), drive("D:\\")]);
  await useDrives.getState().refresh(); // la precedente è tornata: riparte
  expect(listDrives).toHaveBeenCalledTimes(2);
  expect(useDrives.getState().drives).toHaveLength(2);
});

test("folders on removable and optical drives are not watched", () => {
  const drives: Drive[] = [drive("C:\\"), { ...drive("E:\\"), kind: "removable" }, { ...drive("F:\\"), kind: "cdrom" }];
  expect(isUnwatched("e:\\foto", drives)).toBe(true);
  expect(isUnwatched("F:\\", drives)).toBe(true);
  expect(isUnwatched("C:\\e", drives)).toBe(false);
  expect(isUnwatched("drives:", drives)).toBe(false);
});

test("a failed refresh keeps the previous list and releases the guard", async () => {
  useDrives.setState({ drives: [drive("C:\\")] });
  listDrives.mockRejectedValueOnce(new Error("boom"));
  await useDrives.getState().refresh();
  expect(useDrives.getState().drives).toHaveLength(1);
  listDrives.mockResolvedValueOnce([]);
  await useDrives.getState().refresh();
  expect(useDrives.getState().drives).toEqual([]);
});
