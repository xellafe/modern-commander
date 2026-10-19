import { beforeEach, expect, test, vi } from "vitest";

// `Channel` finto con una proprietà `onmessage` vera: l'automock di Vitest scarta il setter.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class { onmessage: ((b: unknown) => void) | undefined; } }));
import { invoke } from "@tauri-apps/api/core";

// `ansiEncoding` memorizza la promessa a livello di modulo: ogni test lo reimporta da zero.
beforeEach(() => {
  vi.resetModules();
  vi.mocked(invoke).mockReset();
});

test("nextRequestId dà id sempre diversi", async () => {
  const { nextRequestId } = await import("./api");
  const a = nextRequestId();
  expect(nextRequestId()).toBeGreaterThan(a);
});

test("nextRequestId non riparte da capo dopo un ricaricamento della pagina", async () => {
  // Il registro delle richieste nel backend sopravvive alla pagina: un id riusato risolverebbe la richiesta sbagliata.
  vi.useFakeTimers({ toFake: ["Date"] });
  try {
    const first = await import("./api");
    first.nextRequestId();
    const last = first.nextRequestId();
    vi.setSystemTime(Date.now() + 1000); // il ricaricamento richiede del tempo
    vi.resetModules();
    const reloaded = await import("./api");
    expect(reloaded.nextRequestId()).toBeGreaterThan(last);
    expect(Number.isSafeInteger(reloaded.nextRequestId())).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

test("readChunk e readFile restituiscono Uint8Array dai byte grezzi", async () => {
  const { readChunk, readFile } = await import("./api");
  vi.mocked(invoke).mockResolvedValue(new Uint8Array([1, 2, 3]).buffer);
  expect(await readChunk("C:\\a", 4, 3)).toEqual(new Uint8Array([1, 2, 3]));
  expect(invoke).toHaveBeenCalledWith("read_chunk", { path: "C:\\a", offset: 4, len: 3 });
  expect(await readFile("C:\\a", 10)).toEqual(new Uint8Array([1, 2, 3]));
  expect(invoke).toHaveBeenCalledWith("read_file", { path: "C:\\a", max: 10 });
});

test("ansiEncoding chiede al backend una volta sola, ma riprova se fallisce", async () => {
  const { ansiEncoding } = await import("./api");
  vi.mocked(invoke).mockRejectedValueOnce(new Error("no")).mockResolvedValue("windows-1250");
  await expect(ansiEncoding()).rejects.toThrow("no");
  expect(await ansiEncoding()).toBe("windows-1250");
  expect(await ansiEncoding()).toBe("windows-1250");
  expect(invoke).toHaveBeenCalledTimes(2);
});

test("scanTree passa i limiti e un canale che inoltra i lotti", async () => {
  const { scanTree } = await import("./api");
  vi.mocked(invoke).mockImplementation(async (_cmd, args) => {
    (args as { onBatch: { onmessage: (b: unknown) => void } }).onBatch.onmessage([{ rel: "a", isDir: false }]);
    return true;
  });
  const batches: unknown[] = [];
  const truncated = await scanTree(3, "C:\\r", { maxDepth: 8, maxEntries: 20000, includeHidden: false }, (b) => batches.push(b));
  expect(truncated).toBe(true);
  expect(batches).toEqual([[{ rel: "a", isDir: false }]]);
  expect(invoke).toHaveBeenCalledWith("scan_tree", expect.objectContaining({
    requestId: 3, root: "C:\\r", maxDepth: 8, maxEntries: 20000, includeHidden: false,
  }));
});

test("nextLineStart e lineStart passano i parametri al backend", async () => {
  const { nextLineStart, lineStart } = await import("./api");
  vi.mocked(invoke).mockResolvedValue(9);
  expect(await nextLineStart(1, "C:\\a", 4, 2, false, () => {})).toBe(9);
  expect(invoke).toHaveBeenCalledWith("next_line_start", {
    requestId: 1, path: "C:\\a", from: 4, unit: 2, bigEndian: false, onProgress: expect.anything(),
  });
  expect(await lineStart(2, "C:\\a", 4, 1, true, () => {})).toBe(9);
  expect(invoke).toHaveBeenCalledWith("line_start", {
    requestId: 2, path: "C:\\a", at: 4, unit: 1, bigEndian: true, onProgress: expect.anything(),
  });
});

test("columnOffsets passa encoding, punti di partenza e target", async () => {
  const { columnOffsets, LINE_END } = await import("./api");
  vi.mocked(invoke).mockResolvedValue([{ offset: 3, column: 3, eol: true }]);
  expect(await columnOffsets(1, "C:\\a", "utf-8", [{ offset: 0, column: 0 }], LINE_END, () => {})).toEqual([
    { offset: 3, column: 3, eol: true },
  ]);
  expect(invoke).toHaveBeenCalledWith("column_offsets", {
    requestId: 1, path: "C:\\a", encoding: "utf-8", from: [{ offset: 0, column: 0 }], target: LINE_END,
    onProgress: expect.anything(),
  });
});

test("thumbnail legge larghezza e altezza little-endian e lascia i pixel RGBA", async () => {
  const { thumbnail } = await import("./api");
  const buf = new Uint8Array([2, 0, 0, 0, 1, 0, 0, 0, 10, 20, 30, 255, 40, 50, 60, 255]).buffer;
  vi.mocked(invoke).mockResolvedValue(buf);
  const t = await thumbnail("C:\\a.jpg", 96);
  expect(invoke).toHaveBeenCalledWith("get_thumbnail", { path: "C:\\a.jpg", size: 96 });
  expect([t.width, t.height]).toEqual([2, 1]);
  expect([...t.data]).toEqual([10, 20, 30, 255, 40, 50, 60, 255]);
});

test("calcHash e listArchive passano id, percorso e parametri ai comandi", async () => {
  const { calcHash, listArchive } = await import("./api");
  vi.mocked(invoke).mockResolvedValueOnce("abc").mockResolvedValueOnce({ entries: [], total: 0, truncated: false });
  expect(await calcHash(5, "C:\\a", "sha256")).toBe("abc");
  expect(invoke).toHaveBeenCalledWith("calc_hash", { requestId: 5, path: "C:\\a", algo: "sha256" });
  expect((await listArchive(6, "C:\\a.zip", 500)).total).toBe(0);
  expect(invoke).toHaveBeenCalledWith("list_archive", { requestId: 6, path: "C:\\a.zip", limit: 500 });
});

test("termSpawn passa solo shell, cartella e dimensioni e inoltra i blocchi come Uint8Array", async () => {
  const { termSpawn } = await import("./api");
  vi.mocked(invoke).mockImplementation(async (_cmd, args) => {
    (args as { onOutput: { onmessage: (b: unknown) => void } }).onOutput.onmessage(new Uint8Array([104, 105]).buffer);
    return 7;
  });
  const chunks: Uint8Array[] = [];
  expect(await termSpawn("cmd", "C:\\x", 80, 24, (b) => chunks.push(b))).toBe(7);
  expect(chunks).toEqual([new Uint8Array([104, 105])]);
  expect(invoke).toHaveBeenCalledWith("term_spawn", expect.objectContaining({ shell: "cmd", cwd: "C:\\x", cols: 80, rows: 24 }));
});

test("termWrite, termResize e termKill chiamano i comandi con l'id", async () => {
  const { termWrite, termResize, termKill } = await import("./api");
  await termWrite(1, "dir\r");
  await termResize(1, 100, 30);
  await termKill(1);
  expect(invoke).toHaveBeenCalledWith("term_write", { id: 1, data: "dir\r" });
  expect(invoke).toHaveBeenCalledWith("term_resize", { id: 1, cols: 100, rows: 30 });
  expect(invoke).toHaveBeenCalledWith("term_kill", { id: 1 });
});
