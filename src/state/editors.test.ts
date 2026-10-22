import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
// Monaco vero non gira in Node: un modello finto con la stessa versione alternativa (undo fino al punto salvato = stessa versione).
vi.mock("../lib/monaco", async (orig) => {
  const real = await orig<typeof import("../lib/monaco")>();
  return {
    ...real,
    loadMonaco: async () => ({
      languages: { getLanguages: () => [{ id: "typescript", extensions: [".ts"] }] },
      editor: { createModel: (text: string, language: string) => new FakeModel(text, language) },
    }),
  };
});
import * as api from "../ipc/api";
import type { FileInfo } from "../ipc/types";
import { useTabViews } from "./tabViews";
import { useUi } from "./ui";
import { blockUnloadIfDirty, decodeForEditor, dirtyEditorCount, editorModel, retryOptions, useEditors } from "./editors";

class FakeModel {
  /** Versione alternativa: cronologia di versioni, l'undo torna a una già vista. */
  history = [1];
  disposed = false;
  private listeners: (() => void)[] = [];
  constructor(public text: string, public language: string) {}
  getValue() { return this.text; }
  getAlternativeVersionId() { return this.history[this.history.length - 1]; }
  onDidChangeContent(cb: () => void) {
    this.listeners.push(cb);
    return { dispose: () => {} };
  }
  dispose() { this.disposed = true; }
  type(text: string) {
    this.text = text;
    this.history.push(this.history.length + 1);
    this.listeners.forEach((l) => l());
  }
  undo() {
    this.history.pop();
    this.text = "contenuto";
    this.listeners.forEach((l) => l());
  }
}

const info = (modifiedMs: number): FileInfo => ({ size: 9, isDir: false, createdMs: 0, modifiedMs, accessedMs: 0, attributes: 0, mtimeReliable: true });
const bytes = (s: string) => new TextEncoder().encode(s);
const model = () => editorModel("t1") as unknown as FakeModel;
const err = (code: string) => ({ code, path: "C:\\a.ts", detail: null });

beforeEach(() => {
  vi.resetAllMocks();
  for (const id of ["t1", "t2"]) useEditors.getState().dispose(id); // modelli e versioni stanno fuori dallo store
  useUi.setState({ dialog: null });
  useTabViews.setState({ notice: null });
  vi.mocked(api.ansiEncoding).mockResolvedValue("windows-1252");
  vi.mocked(api.readFile).mockResolvedValue(bytes("contenuto"));
  vi.mocked(api.fileInfo).mockResolvedValue(info(100));
});

test("decodeForEditor", () => {
  expect(decodeForEditor(new Uint8Array([0xef, 0xbb, 0xbf, 0x61]), "windows-1252")).toEqual({ text: "a", encoding: "utf-8", bom: true, readOnly: false });
  expect(decodeForEditor(new Uint8Array([0x70, 0xe8]), "windows-1252")).toMatchObject({ text: "pè", encoding: "windows-1252" });
  expect(decodeForEditor(new Uint8Array([1, 0, 2]), "windows-1252").readOnly).toBe(true);
  // UTF-16 con BOM: gli zeri sono normali, il file si modifica.
  expect(decodeForEditor(new Uint8Array([0xff, 0xfe, 0x61, 0]), "windows-1252")).toEqual({ text: "a", encoding: "utf-16le", bom: true, readOnly: false });
});

test("decodeForEditor opens read-only what it cannot decode without losing data", () => {
  // UTF-8 con BOM ma con un byte non valido dopo: la decodifica lo sostituirebbe con U+FFFD.
  const bad = decodeForEditor(new Uint8Array([0xef, 0xbb, 0xbf, 0x61, 0xff]), "windows-1252");
  expect(bad).toMatchObject({ encoding: "utf-8", bom: true, readOnly: true });
  expect(bad.text).toBe("a\ufffd"); // il testo si mostra comunque
  expect(decodeForEditor(new Uint8Array([0x70, 0xe8]), "windows-1252").readOnly).toBe(false);
  // Byte senza corrispondenza in una codifica ANSI a byte singolo (0xD2 non esiste in windows-1253) e sequenza doppia non valida.
  expect(decodeForEditor(new Uint8Array([0x61, 0xd2]), "windows-1253").readOnly).toBe(true);
  expect(decodeForEditor(new Uint8Array([0x61, 0x81, 0x20]), "shift_jis").readOnly).toBe(true);
  // UTF-16 di lunghezza dispari o con surrogato isolato.
  expect(decodeForEditor(new Uint8Array([0xff, 0xfe, 0x61, 0x00, 0x62]), "windows-1252").readOnly).toBe(true);
  expect(decodeForEditor(new Uint8Array([0xff, 0xfe, 0x00, 0xd8, 0x61, 0x00]), "windows-1252").readOnly).toBe(true);
});

test("open reads the file with the editor limit and keeps the model outside the store", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  expect(api.readFile).toHaveBeenCalledWith("C:\\a.ts", 32 * 1024 * 1024);
  expect(useEditors.getState().docs.t1).toEqual({ path: "C:\\a.ts", encoding: "utf-8", bom: false, modifiedMs: 100, dirty: false, readOnly: false });
  expect(model()).toMatchObject({ text: "contenuto", language: "typescript" });
  await useEditors.getState().open("t1", "C:\\a.ts"); // ritorno alla scheda: niente seconda lettura
  expect(api.readFile).toHaveBeenCalledTimes(1);
});

test("open propagates TOO_LARGE and leaves nothing behind", async () => {
  vi.mocked(api.readFile).mockRejectedValue(err("TOO_LARGE"));
  await expect(useEditors.getState().open("t1", "C:\\big.log")).rejects.toMatchObject({ code: "TOO_LARGE" });
  expect(useEditors.getState().docs.t1).toBeUndefined();
  expect(editorModel("t1")).toBeUndefined();
});

test("a tab closed while the file is loading does not come back", async () => {
  const opening = useEditors.getState().open("t1", "C:\\a.ts");
  useEditors.getState().dispose("t1");
  await opening;
  expect(useEditors.getState().docs.t1).toBeUndefined();
  expect(editorModel("t1")).toBeUndefined();
});

test("blockUnloadIfDirty: la ricarica della pagina si ferma solo con modifiche non salvate", async () => {
  const ev = () => ({ preventDefault: vi.fn(), returnValue: undefined as unknown });
  const clean = ev();
  blockUnloadIfDirty(clean);
  expect(clean.preventDefault).not.toHaveBeenCalled();
  await useEditors.getState().open("t1", "C:\\a.ts");
  model().type("uno");
  const dirty = ev();
  blockUnloadIfDirty(dirty);
  expect(dirty.preventDefault).toHaveBeenCalled();
  expect(dirty.returnValue).toBe(""); // WebView2 chiede la conferma solo con returnValue impostato
});

test("dirty follows the version of the last save: undo back to the saved point is clean", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  model().type("uno");
  expect(useEditors.getState().docs.t1.dirty).toBe(true);
  expect(dirtyEditorCount()).toBe(1);
  model().undo();
  expect(useEditors.getState().docs.t1.dirty).toBe(false);
  expect(dirtyEditorCount()).toBe(0);
  // Dopo un salvataggio il punto "pulito" si sposta; l'undo oltre quel punto sporca di nuovo.
  vi.mocked(api.writeTextFile).mockResolvedValue(info(200));
  model().type("due");
  await useEditors.getState().save("t1");
  expect(useEditors.getState().docs.t1).toMatchObject({ dirty: false, modifiedMs: 200 });
  model().undo();
  expect(useEditors.getState().docs.t1.dirty).toBe(true);
});

test("save sends the text with the expected date, or without it when forced, and a later edit stays dirty", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  vi.mocked(api.writeTextFile).mockImplementation(async () => {
    model().type("scritto durante il salvataggio");
    return info(300);
  });
  model().type("nuovo");
  await useEditors.getState().save("t1");
  expect(api.writeTextFile).toHaveBeenLastCalledWith("C:\\a.ts", "nuovo", "utf-8", false, 100);
  expect(useEditors.getState().docs.t1).toMatchObject({ dirty: true, modifiedMs: 300 });
  vi.mocked(api.writeTextFile).mockResolvedValue(info(400));
  await useEditors.getState().save("t1", { force: true, encoding: "windows-1250" });
  expect(api.writeTextFile).toHaveBeenLastCalledWith("C:\\a.ts", "scritto durante il salvataggio", "windows-1250", false, null);
  expect(useEditors.getState().docs.t1).toMatchObject({ dirty: false, encoding: "windows-1250", modifiedMs: 400 });
});

test("CHANGED_ON_DISK and UNMAPPABLE_CHARS open the save error dialog, other errors a notice; the tab stays dirty", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  model().type("nuovo");
  for (const code of ["CHANGED_ON_DISK", "UNMAPPABLE_CHARS"] as const) {
    vi.mocked(api.writeTextFile).mockRejectedValueOnce(err(code));
    await useEditors.getState().save("t1");
    expect(useUi.getState().dialog).toEqual({ kind: "saveError", tabId: "t1", code, opts: {} });
    useUi.getState().closeDialog();
  }
  vi.mocked(api.writeTextFile).mockRejectedValueOnce(err("ACCESS_DENIED"));
  await useEditors.getState().save("t1");
  expect(useUi.getState().dialog).toBeNull();
  expect(useTabViews.getState().notice).toMatchObject({ tabId: "t1", code: "ACCESS_DENIED" });
  expect(useEditors.getState().docs.t1.dirty).toBe(true);
});

test("a read-only (binary) document is never written", async () => {
  vi.mocked(api.readFile).mockResolvedValue(new Uint8Array([1, 0, 2]));
  await useEditors.getState().open("t1", "C:\\a.exe");
  expect(useEditors.getState().docs.t1.readOnly).toBe(true);
  await useEditors.getState().save("t1");
  expect(api.writeTextFile).not.toHaveBeenCalled();
});

test("dispose frees the model and the document", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  const m = model();
  useEditors.getState().dispose("t1");
  expect(m.disposed).toBe(true);
  expect(useEditors.getState().docs.t1).toBeUndefined();
  expect(editorModel("t1")).toBeUndefined();
});

test("retryOptions merges the correction with the options of the attempt that failed", () => {
  expect(retryOptions({}, "CHANGED_ON_DISK")).toEqual({ force: true });
  expect(retryOptions({}, "UNMAPPABLE_CHARS")).toEqual({ encoding: "utf-8" });
  // File cambiato E caratteri non rappresentabili: le due correzioni si sommano, in qualunque ordine.
  expect(retryOptions({ force: true }, "UNMAPPABLE_CHARS")).toEqual({ force: true, encoding: "utf-8" });
  expect(retryOptions({ encoding: "utf-8" }, "CHANGED_ON_DISK")).toEqual({ encoding: "utf-8", force: true });
});

test("the save error dialog carries the options of the failed attempt, so the corrections combine", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  model().type("nuovo");
  vi.mocked(api.writeTextFile).mockRejectedValueOnce(err("CHANGED_ON_DISK"));
  await useEditors.getState().save("t1");
  const first = useUi.getState().dialog as Extract<NonNullable<ReturnType<typeof useUi.getState>["dialog"]>, { kind: "saveError" }>;
  useUi.getState().closeDialog();
  // "Sovrascrivi": il tentativo con force trova caratteri non rappresentabili.
  vi.mocked(api.writeTextFile).mockRejectedValueOnce(err("UNMAPPABLE_CHARS"));
  await useEditors.getState().save("t1", retryOptions(first.opts, first.code));
  const second = useUi.getState().dialog as typeof first;
  expect(second).toMatchObject({ code: "UNMAPPABLE_CHARS", opts: { force: true } });
  useUi.getState().closeDialog();
  // "Salva in UTF-8" riparte con force E con la nuova codifica.
  vi.mocked(api.writeTextFile).mockResolvedValueOnce(info(500));
  await useEditors.getState().save("t1", retryOptions(second.opts, second.code));
  expect(api.writeTextFile).toHaveBeenLastCalledWith("C:\\a.ts", "nuovo", "utf-8", false, null);
  expect(useEditors.getState().docs.t1).toMatchObject({ dirty: false, encoding: "utf-8", modifiedMs: 500 });
});

/** Una scrittura che si risolve a comando, per vedere quante ne girano insieme. */
function controlledWrites() {
  const gates: (() => void)[] = [];
  let running = 0;
  let maxRunning = 0;
  vi.mocked(api.writeTextFile).mockImplementation(async () => {
    maxRunning = Math.max(maxRunning, ++running);
    await new Promise<void>((r) => gates.push(r));
    running--;
    return info(900);
  });
  return { gates, max: () => maxRunning };
}
const tick = () => new Promise((r) => setTimeout(r, 0));

test("saves of the same file never run in parallel, even from two tabs, and repeated plain saves of a tab merge", async () => {
  await useEditors.getState().open("t1", "C:\\a.ts");
  await useEditors.getState().open("t2", "C:\\A.TS"); // lo stesso file (il nome non distingue le maiuscole)
  const w = controlledWrites();
  const saves = [useEditors.getState().save("t1")];
  await tick(); // il primo è partito e non è finito
  saves.push(
    useEditors.getState().save("t1"), // in coda
    useEditors.getState().save("t1"), // identico a quello in coda: si fonde
    useEditors.getState().save("t1"),
    useEditors.getState().save("t2"), // altra scheda, stesso file: dopo
  );
  await tick();
  expect(api.writeTextFile).toHaveBeenCalledTimes(1);
  for (let i = 0; i < 3; i++) {
    w.gates.shift()!();
    await tick();
  }
  await Promise.all(saves);
  expect(api.writeTextFile).toHaveBeenCalledTimes(3); // t1, t1 (una per tutte le richieste in coda), t2
  expect(w.max()).toBe(1);
});

test("saves of different files do run independently", async () => {
  vi.mocked(api.readFile).mockResolvedValue(bytes("x"));
  await useEditors.getState().open("t1", "C:\\a.ts");
  await useEditors.getState().open("t2", "C:\\b.ts");
  const w = controlledWrites();
  const both = [useEditors.getState().save("t1"), useEditors.getState().save("t2")];
  await tick();
  expect(api.writeTextFile).toHaveBeenCalledTimes(2);
  w.gates.splice(0).forEach((g) => g());
  await Promise.all(both);
});
