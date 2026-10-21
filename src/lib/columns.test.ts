import { expect, test } from "vitest";
import {
  DEFAULT_COLUMNS,
  driveSpan,
  gridTemplate,
  moveColumn,
  normalizeColumns,
  resizeColumn,
  toggleColumn,
  typeKey,
  visibleColumns,
  type ColumnConfig,
} from "./columns";

test("gridTemplate dei default è la griglia di oggi col minimo del Nome", () => {
  expect(gridTemplate(visibleColumns(DEFAULT_COLUMNS, false))).toBe("16px minmax(80px, 1fr) 52px 72px 112px 36px");
  expect(gridTemplate(visibleColumns(DEFAULT_COLUMNS, true))).toBe("16px minmax(80px, 1fr) 72px 112px 36px");
});

test("normalizeColumns", () => {
  expect(normalizeColumns(undefined)).toEqual(DEFAULT_COLUMNS);
  const n = normalizeColumns([
    { id: "type", visible: true, width: 200 }, { id: "bogus", visible: true, width: 10 },
    { id: "type", visible: false, width: 50 }, { id: "name", visible: false, width: 300 },
    { id: "size", visible: true, width: "x" }, { id: "ext", visible: true, width: 3 },
  ]);
  expect(n.slice(0, 4)).toEqual([
    { id: "type", visible: true, width: 200 }, { id: "name", visible: true, width: null },
    { id: "size", visible: true, width: 72 }, { id: "ext", visible: true, width: 28 },
  ]);
  expect(n.map((c) => c.id).sort()).toEqual(DEFAULT_COLUMNS.map((c) => c.id).sort()); // mancanti in fondo coi default
  expect(n.slice(4).map((c) => c.id)).toEqual(["modified", "created", "accessed", "attributes"]);
});

test("moveColumn e toggleColumn", () => {
  const ids = (c: ColumnConfig[]) => c.map((x) => x.id);
  expect(ids(moveColumn(DEFAULT_COLUMNS, "size", "name")).slice(0, 2)).toEqual(["size", "name"]);
  expect(ids(moveColumn(DEFAULT_COLUMNS, "name", null)).slice(-1)).toEqual(["name"]);
  expect(toggleColumn(DEFAULT_COLUMNS, "name")).toEqual(DEFAULT_COLUMNS);
  expect(toggleColumn(DEFAULT_COLUMNS, "type").find((c) => c.id === "type")!.visible).toBe(true);
});

test("resizeColumn clamps", () => {
  expect(resizeColumn(DEFAULT_COLUMNS, "size", -500).find((c) => c.id === "size")!.width).toBe(28);
  expect(resizeColumn(DEFAULT_COLUMNS, "size", 90.6).find((c) => c.id === "size")!.width).toBe(91);
  expect(resizeColumn(DEFAULT_COLUMNS, "name", 300)).toEqual(DEFAULT_COLUMNS);
});

test("typeKey", () => {
  expect(typeKey(".gitignore", false)).toBe("");
  expect(typeKey("archive.tar.GZ", false)).toBe("gz");
  expect(typeKey("README", false)).toBe("");
  expect(typeKey("v1.2", true)).toBe("/");
});

test("driveSpan", () => {
  expect(driveSpan(["name", "ext", "size"])).toBe("3 / -1");
  expect(driveSpan(["size", "ext", "name"])).toBe("2 / 4");
  expect(driveSpan(["ext", "name", "size"])).toBe("4 / -1");
  expect(driveSpan(["name"])).toBeNull();
});
