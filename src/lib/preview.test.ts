import { expect, test } from "vitest";
import { looksBinary, previewKind } from "./preview";

const enc = (s: string) => new TextEncoder().encode(s);

test("preview kinds", () => {
  expect(previewKind("a.PNG")).toBe("image"); expect(previewKind("a.tif")).toBeNull();
  expect(previewKind("a.tar.gz")).toBe("archive"); expect(previewKind("a.gz")).toBeNull();
  expect(previewKind("r.md")).toBe("markdown"); expect(previewKind("a.pdf")).toBe("pdf");
});

test("preview kinds: video, audio, archivi e nomi senza estensione", () => {
  expect(previewKind("a.mp4")).toBe("video"); expect(previewKind("a.mkv")).toBeNull();
  expect(previewKind("a.opus")).toBe("audio"); expect(previewKind("a.wma")).toBeNull();
  expect(previewKind("a.zip")).toBe("archive"); expect(previewKind("a.rar")).toBeNull();
  expect(previewKind("A.TAR.GZ")).toBe("archive");
  expect(previewKind("Makefile")).toBeNull(); expect(previewKind(".md")).toBeNull();
});

test("binary detection", () => {
  expect(looksBinary(new Uint8Array([0x41, 0, 0x42]))).toBe(true);
  expect(looksBinary(new Uint8Array([0xff, 0xfe, 0x41, 0]))).toBe(false);
  expect(looksBinary(enc("testo"))).toBe(false);
});

test("binary detection: BOM UTF-16BE e campione vuoto", () => {
  expect(looksBinary(new Uint8Array([0xfe, 0xff, 0, 0x41]))).toBe(false);
  expect(looksBinary(new Uint8Array(0))).toBe(false);
});
