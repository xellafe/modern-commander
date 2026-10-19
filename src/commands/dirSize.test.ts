import { beforeAll, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { DirSize, DirSizeEvent, Entry } from "../ipc/types";
import { useLayout } from "../state/layout";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { registerNavigationCommands } from "./navigation";
import { executeCommand } from "./registry";

const listDir = vi.mocked(api.listDir);
const calc = vi.mocked(api.calcDirSize);
const e = (name: string, isDir = false): Entry => ({ name, isDir, size: 0, modifiedMs: 0, attributes: 0, isSymlink: false });
const ctx = () => activeTabContext();
const size = (bytes: number, skipped = 0): DirSize => ({ bytes, files: 1, dirs: 0, skipped });

// L'ascoltatore si registra una sola volta per modulo: `emit` resta quello della prima richiesta.
let emit!: (ev: DirSizeEvent) => void;
const reply = (call: number, result: DirSize | null, error: DirSizeEvent["error"] = null) => {
  const [requestId, path] = calc.mock.calls[call];
  emit({ requestId, path, result, error });
};
const idOf = (call: number) => calc.mock.calls[call][0];

beforeAll(registerNavigationCommands);
beforeEach(async () => {
  vi.resetAllMocks();
  vi.mocked(api.watchDir).mockResolvedValue();
  vi.mocked(api.unwatchDir).mockResolvedValue();
  vi.mocked(api.calcDirSize).mockResolvedValue();
  vi.mocked(api.cancelRequest).mockResolvedValue();
  let lastId = 0;
  vi.mocked(api.nextRequestId).mockImplementation(() => ++lastId);
  vi.mocked(api.onDirSize).mockImplementation(async (cb) => {
    emit = cb;
    return () => {};
  });
  useLayout.setState({
    groups: [{ id: "g1", tabIds: ["t1"], activeTabId: "t1" }],
    tabs: { t1: { id: "t1", path: "C:\\" } },
    activeGroupId: "g1",
    activationOrder: ["g1"],
  });
  useTabViews.setState({ views: {}, notice: null });
  listDir.mockResolvedValueOnce([e("a", true), e("b", true), e("f.txt")]);
  await useTabViews.getState().load("t1", "C:\\x"); // righe: .., a, b, f.txt
});

const cursorTo = (name: string) =>
  useTabViews.getState().update("t1", (v) => ({ ...v, cursor: v.rows.findIndex((r) => r.name === name) }));

test("Spazio su una cartella la marca, calcola la dimensione e non sposta il cursore", async () => {
  cursorTo("a");
  await executeCommand("mc_MarkOrSize");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(1));
  expect(calc.mock.calls[0][1]).toBe("C:\\x\\a");
  expect([...ctx().view.marked]).toEqual(["a"]);
  expect(ctx().cursorRow?.name).toBe("a");
  expect([...ctx().view.sizing]).toEqual(["a"]);

  reply(0, size(5, 2));
  await vi.waitFor(() => expect(ctx().view.sizing.size).toBe(0));
  expect(ctx().view.dirSizes).toEqual({ a: 5 });
  expect(ctx().view.skipped).toEqual({ a: 2 });
});

test("smarcando la cartella la dimensione resta e non si ricalcola", async () => {
  cursorTo("a");
  await executeCommand("mc_MarkOrSize");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(1));
  reply(0, size(5));
  await vi.waitFor(() => expect(ctx().view.sizing.size).toBe(0));
  await executeCommand("mc_MarkOrSize"); // smarca
  expect(ctx().view.marked.size).toBe(0);
  expect(ctx().view.dirSizes).toEqual({ a: 5 });
  expect(calc).toHaveBeenCalledTimes(1);
});

test("Spazio su un file lo marca e basta; su '..' non fa niente", async () => {
  cursorTo("f.txt");
  await executeCommand("mc_MarkOrSize");
  expect([...ctx().view.marked]).toEqual(["f.txt"]);
  cursorTo("..");
  await executeCommand("mc_MarkOrSize");
  expect([...ctx().view.marked]).toEqual(["f.txt"]);
  expect(calc).not.toHaveBeenCalled();
});

test("l'errore sulla radice mostra un avviso e chiude il calcolo", async () => {
  cursorTo("a");
  await executeCommand("mc_MarkOrSize");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(1));
  reply(0, null, { code: "ACCESS_DENIED", path: "C:\\x\\a", detail: null });
  await vi.waitFor(() => expect(ctx().view.sizing.size).toBe(0));
  expect(useTabViews.getState().notice?.code).toBe("ACCESS_DENIED");
  expect(ctx().view.dirSizes).toEqual({});
});

test("con l'ordinamento per dimensione il risultato riordina le righe", async () => {
  await executeCommand("cm_SrcBySize"); // crescente: a e b a 0
  await executeCommand("cm_SrcBySize"); // decrescente
  expect(ctx().view.rows.map((r) => r.name)).toEqual(["..", "a", "b", "f.txt"]);
  cursorTo("b");
  await executeCommand("mc_MarkOrSize");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(1));
  reply(0, size(20));
  await vi.waitFor(() => expect(ctx().view.dirSizes).toEqual({ b: 20 }));
  expect(ctx().view.rows.map((r) => r.name)).toEqual(["..", "b", "a", "f.txt"]);
  expect(ctx().cursorRow?.name).toBe("b"); // il cursore segue la riga
});

test("cm_CountDirContent salta le cartelle già calcolate e ne misura al massimo 4 per volta", async () => {
  listDir.mockResolvedValueOnce(["d1", "d2", "d3", "d4", "d5", "d6"].map((n) => e(n, true)).concat(e("f")));
  await useTabViews.getState().load("t1", "C:\\y");
  useTabViews.getState().update("t1", (v) => ({ ...v, dirSizes: { d1: 1 } }));
  await executeCommand("cm_CountDirContent");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(4));
  expect([...ctx().view.sizing]).toEqual(["d2", "d3", "d4", "d5", "d6"]); // tutte in attesa, 4 in volo
  expect(calc.mock.calls.map((c) => c[1])).toEqual(["C:\\y\\d2", "C:\\y\\d3", "C:\\y\\d4", "C:\\y\\d5"]);

  reply(0, size(2));
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(5));
  expect(calc.mock.calls[4][1]).toBe("C:\\y\\d6");
  for (const i of [1, 2, 3, 4]) reply(i, size(i + 2));
  await vi.waitFor(() => expect(ctx().view.sizing.size).toBe(0));
  expect(ctx().view.dirSizes).toEqual({ d1: 1, d2: 2, d3: 3, d4: 4, d5: 5, d6: 6 });
  await executeCommand("cm_CountDirContent"); // niente da fare
  expect(calc).toHaveBeenCalledTimes(5);
});

test("cambiare cartella annulla i calcoli in corso e scarta le risposte tardive", async () => {
  await executeCommand("cm_CountDirContent");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(2));
  listDir.mockResolvedValueOnce([e("a", true)]);
  await useTabViews.getState().load("t1", "C:\\z");
  expect(api.cancelRequest).toHaveBeenCalledTimes(2);
  expect(vi.mocked(api.cancelRequest).mock.calls.map((c) => c[0]).sort()).toEqual([idOf(0), idOf(1)].sort());
  expect(ctx().view.sizing.size).toBe(0);
  reply(0, size(5)); // arriva dopo l'annullamento
  await Promise.resolve();
  expect(ctx().view.dirSizes).toEqual({});
});

test("ricaricare la stessa cartella non annulla il calcolo", async () => {
  cursorTo("a");
  await executeCommand("mc_MarkOrSize");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(1));
  listDir.mockResolvedValueOnce([e("a", true), e("b", true)]);
  await useTabViews.getState().reload("t1");
  expect(api.cancelRequest).not.toHaveBeenCalled();
  reply(0, size(7));
  await vi.waitFor(() => expect(ctx().view.dirSizes).toEqual({ a: 7 }));
});

test("chiudere la scheda annulla i calcoli", async () => {
  cursorTo("a");
  await executeCommand("mc_MarkOrSize");
  await vi.waitFor(() => expect(calc).toHaveBeenCalledTimes(1));
  useTabViews.getState().dispose("t1");
  expect(api.cancelRequest).toHaveBeenCalledWith(idOf(0));
});

test("i comandi hanno il titolo in entrambe le lingue", async () => {
  const en = (await import("../i18n/en.json")).default as { commands: Record<string, string> };
  const it = (await import("../i18n/it.json")).default as { commands: Record<string, string> };
  for (const id of ["mc_MarkOrSize", "cm_CountDirContent"]) {
    expect(en.commands[id], id).toBeTruthy();
    expect(it.commands[id], id).toBeTruthy();
  }
});
