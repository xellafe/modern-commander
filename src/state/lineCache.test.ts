import { expect, test } from "vitest";
import { disposeLineCache, lineCache } from "./lineCache";

test("lineCache resets marks and widths on encoding change", () => {
  const c = lineCache("t1", "windows-1252");
  c.next.set(0, 10);
  c.fwd.set(0, 10);
  c.back.set(10, 0);
  c.marks.set(0, [{ offset: 5, column: 5 }]);
  c.widths.set(0, 9);
  const u = lineCache("t1", "utf-8");
  expect(u.next.get(0)).toBe(10);
  expect(u.fwd.get(0)).toBe(10);
  expect(u.back.get(10)).toBe(0);
  expect(u.marks.size).toBe(0);
  expect(u.widths.size).toBe(0);
  expect(u.encoding).toBe("utf-8");
  const w = lineCache("t1", "utf-16le");
  expect(w.next.size + w.fwd.size + w.back.size).toBe(0);
  // stessa unità, altro ordine dei byte: gli LF trovati non valgono più
  w.next.set(0, 10);
  w.fwd.set(0, 10);
  w.back.set(10, 0);
  const be = lineCache("t1", "utf-16be");
  expect(be.next.size + be.fwd.size + be.back.size).toBe(0);
  disposeLineCache("t1");
});

test("disposeLineCache drops the tab", () => {
  const c = lineCache("t2", "utf-8");
  c.next.set(0, 1);
  disposeLineCache("t2");
  expect(lineCache("t2", "utf-8").next.size).toBe(0);
  disposeLineCache("t2");
});
