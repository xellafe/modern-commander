import { expect, test } from "vitest";
import { pushRecent, useRecentDirs } from "./recentDirs";

test("pushRecent mette in testa e toglie i doppioni senza distinzione di maiuscole", () => {
  expect(pushRecent(["C:\\a", "C:\\b"], "c:\\B")).toEqual(["c:\\B", "C:\\a"]);
  expect(pushRecent(["C:\\a"], "C:\\a")).toEqual(["C:\\a"]);
});

test("pushRecent tiene al massimo 50 cartelle, le più recenti", () => {
  const out = pushRecent(Array.from({ length: 50 }, (_, i) => `C:\\${i}`), "D:\\x");
  expect(out).toHaveLength(50);
  expect(out[0]).toBe("D:\\x");
  expect(out).not.toContain("C:\\49");
  expect(pushRecent(["C:\\a", "C:\\b"], "C:\\c", 2)).toEqual(["C:\\c", "C:\\a"]);
});

test("useRecentDirs.push aggiorna l'elenco", () => {
  useRecentDirs.setState({ dirs: [] });
  useRecentDirs.getState().push("C:\\a");
  useRecentDirs.getState().push("C:\\b");
  expect(useRecentDirs.getState().dirs).toEqual(["C:\\b", "C:\\a"]);
});
