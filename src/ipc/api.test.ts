import { beforeEach, expect, test, vi } from "vitest";

vi.mock("@tauri-apps/api/core");
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

test("readChunk e readFile restituiscono Uint8Array dai byte grezzi", async () => {
  const { readChunk, readFile } = await import("./api");
  vi.mocked(invoke).mockResolvedValue(new Uint8Array([1, 2, 3]).buffer);
  expect(await readChunk("C:\a", 4, 3)).toEqual(new Uint8Array([1, 2, 3]));
  expect(invoke).toHaveBeenCalledWith("read_chunk", { path: "C:\a", offset: 4, len: 3 });
  expect(await readFile("C:\a", 10)).toEqual(new Uint8Array([1, 2, 3]));
  expect(invoke).toHaveBeenCalledWith("read_file", { path: "C:\a", max: 10 });
});

test("ansiEncoding chiede al backend una volta sola, ma riprova se fallisce", async () => {
  const { ansiEncoding } = await import("./api");
  vi.mocked(invoke).mockRejectedValueOnce(new Error("no")).mockResolvedValue("windows-1250");
  await expect(ansiEncoding()).rejects.toThrow("no");
  expect(await ansiEncoding()).toBe("windows-1250");
  expect(await ansiEncoding()).toBe("windows-1250");
  expect(invoke).toHaveBeenCalledTimes(2);
});
