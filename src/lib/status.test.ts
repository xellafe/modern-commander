import i18next from "i18next";
import { expect, test } from "vitest";
import en from "../i18n/en.json";
import it from "../i18n/it.json";
import type { Drive, JobView } from "../ipc/types";
import { activeJobs } from "../state/jobs";
import type { Row } from "../state/tabView";
import { driveOf, summarize, summaryText } from "./status";

const file = (name: string, size: number): Row => ({ name, isDir: false, size, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false });
const dir = (name: string): Row => ({ name, isDir: true, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false });
const parent: Row = { ...dir(".."), isParent: true };
const rows = [parent, dir("src"), dir("docs"), file("a.txt", 1000), file("b.txt", 2000)];

// t finto: mostra chiave e parametri
const t = ((k: string, o: object) => `${k} ${JSON.stringify(o)}`) as never;

test("senza marcati: file e cartelle della vista, '..' esclusa", () => {
  const s = summarize({ rows, marked: new Set(), dirSizes: {} });
  expect(s).toEqual({ kind: "contents", files: 2, folders: 2 });
  expect(summaryText(s, t, "it")).toBe('status.files {"count":2}, status.folders {"count":2}');
});

test("marcati: numero e somma delle dimensioni dei file", () => {
  const s = summarize({ rows, marked: new Set(["a.txt", "b.txt"]), dirSizes: {} });
  expect(s).toEqual({ kind: "marked", count: 2, bytes: 3000, approx: false });
  expect(summaryText(s, t, "en")).toBe('status.selection {"count":2,"size":"2.9 KB"}');
});

test("cartelle marcate: misurate si sommano, non misurate rendono il totale approssimato", () => {
  const sized = summarize({ rows, marked: new Set(["src", "a.txt"]), dirSizes: { src: 500 } });
  expect(sized).toEqual({ kind: "marked", count: 2, bytes: 1500, approx: false });
  const unsized = summarize({ rows, marked: new Set(["src", "docs", "a.txt"]), dirSizes: { src: 500 } });
  expect(unsized).toEqual({ kind: "marked", count: 3, bytes: 1500, approx: true });
  expect(summaryText(unsized, t, "en")).toContain("status.selectionApprox");
});

test("un nome come 'constructor' non conta come dimensione nota", () => {
  const s = summarize({ rows: [dir("constructor")], marked: new Set(["constructor"]), dirSizes: {} });
  expect(s).toMatchObject({ bytes: 0, approx: true });
});

test("marcati non più presenti nelle righe: si torna al riepilogo della vista", () => {
  expect(summarize({ rows, marked: new Set(["x"]), dirSizes: {} }).kind).toBe("contents");
});

const drive = (root: string, ready = true): Drive => ({ root, label: "", kind: "fixed", total: 10, free: 5, ready });

test("driveOf: radice confrontata senza maiuscole", () => {
  const ds = [drive("C:\\"), drive("D:\\")];
  expect(driveOf("d:\\Backup\\x", ds)?.root).toBe("D:\\");
  expect(driveOf("C:\\", ds)?.root).toBe("C:\\");
  expect(driveOf("C:", ds)?.root).toBe("C:\\");
});

test("driveOf: UNC, pseudo-percorso delle unità e unità non pronta non hanno spazio libero", () => {
  const ds = [drive("C:\\"), drive("E:\\", false)];
  expect(driveOf("\\\\server\\share\\a", ds)).toBeNull();
  expect(driveOf("drives:", ds)).toBeNull();
  expect(driveOf("E:\\", ds)).toBeNull();
  expect(driveOf("Z:\\", ds)).toBeNull();
});

test("activeJobs: in corso, in attesa e in pausa nell'ordine della coda; accodati e finiti no", () => {
  const j = (id: number, type: JobView["state"]["type"]) => ({ id, state: { type } }) as JobView;
  const jobs = { 1: j(1, "done"), 2: j(2, "running"), 3: j(3, "queued"), 4: j(4, "paused") };
  expect(activeJobs([1, 2, 3, 4], jobs).map((x) => x.id)).toEqual([2, 4]);
  expect(activeJobs([4, 2], jobs).map((x) => x.id)).toEqual([4, 2]);
});

test("summaryText con le traduzioni reali (en/it, plurali e forma approssimata)", async () => {
  const make = async (lng: "en" | "it") => {
    const i = i18next.createInstance();
    await i.init({ lng, resources: { en: { translation: en }, it: { translation: it } }, interpolation: { escapeValue: false } });
    return i.t;
  };
  const [tEn, tIt] = [await make("en"), await make("it")];
  const one = { kind: "marked", count: 1, bytes: 1500, approx: false } as const;
  const two = { kind: "marked", count: 2, bytes: 1500, approx: true } as const;
  expect(summaryText(one, tIt, "it")).toBe("1 selezionato · 1,5 KB");
  expect(summaryText(two, tIt, "it")).toBe("2 selezionati · ≥ 1,5 KB");
  expect(summaryText(one, tEn, "en")).toBe("1 selected · 1.5 KB");
  expect(summaryText(two, tEn, "en")).toBe("2 selected · ≥ 1.5 KB");
  expect(summaryText({ kind: "contents", files: 1, folders: 3 }, tIt, "it")).toBe("1 file, 3 cartelle");
  expect(summaryText({ kind: "contents", files: 2, folders: 1 }, tEn, "en")).toBe("2 files, 1 folder");
});
