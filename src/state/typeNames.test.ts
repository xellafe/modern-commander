import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import { ensureTypeNames, useTypeNames } from "./typeNames";

const typeNames = vi.mocked(api.typeNames);

beforeEach(() => {
  vi.resetAllMocks();
  useTypeNames.setState({ names: {} });
});

test("fetches only missing keys in one call", async () => {
  typeNames.mockResolvedValueOnce(["Documento di testo", "Cartella di file"]);
  expect(await ensureTypeNames(["txt", "/", "txt"])).toBe(true);
  expect(typeNames).toHaveBeenCalledWith(["txt", "/"]);
  expect(useTypeNames.getState().names).toEqual({ txt: "Documento di testo", "/": "Cartella di file" });

  expect(await ensureTypeNames(["/", "txt"])).toBe(false);
  expect(typeNames).toHaveBeenCalledTimes(1);

  typeNames.mockResolvedValueOnce(["Immagine PNG"]);
  expect(await ensureTypeNames(["txt", "png"])).toBe(true);
  expect(typeNames).toHaveBeenLastCalledWith(["png"]);
});

test("concurrent calls do not duplicate keys", async () => {
  let resolve!: (v: string[]) => void;
  typeNames.mockReturnValueOnce(new Promise((r) => (resolve = r))).mockResolvedValueOnce(["Immagine PNG"]);
  const a = ensureTypeNames(["txt"]);
  const b = ensureTypeNames(["txt", "png"]);
  resolve(["Documento di testo"]);
  await Promise.all([a, b]);
  expect(typeNames.mock.calls).toEqual([[["txt"]], [["png"]]]);
});

test("failure caches empty names", async () => {
  typeNames.mockRejectedValueOnce(new Error("boom"));
  expect(await ensureTypeNames(["txt"])).toBe(true);
  expect(useTypeNames.getState().names).toEqual({ txt: "" });
  expect(await ensureTypeNames(["txt"])).toBe(false);
  expect(typeNames).toHaveBeenCalledTimes(1);
});

test("keys named like Object.prototype members are fetched", async () => {
  typeNames.mockResolvedValueOnce(["File CONSTRUCTOR"]);
  await ensureTypeNames(["constructor"]);
  expect(typeNames).toHaveBeenCalledWith(["constructor"]);
});
