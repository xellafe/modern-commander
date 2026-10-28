import { beforeEach, expect, test } from "vitest";
import { alignTop, initialViewerState, nativeMode, needsLocalCopy, skipsSample, useViewers } from "./viewers";

const W = "windows-1252";

test("images the WebView cannot decode open as image: ImagePane falls back to the shell preview", () => {
  const tiff = new Uint8Array([0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00]);
  expect(nativeMode("a.tif")).toBe("image");
  expect(nativeMode("a.HEIC")).toBe("image");
  expect(initialViewerState("a.tiff", tiff, W, true).mode).toBe("image");
  expect(nativeMode("a.zip")).toBeNull(); // le altre estensioni non cambiano
});

test("initial mode and encoding", () => {
  const txt = new TextEncoder().encode("ciao");
  expect(initialViewerState("a.png", txt, W, true).mode).toBe("image");
  expect(initialViewerState("a.dat", new Uint8Array([1, 0, 2]), W, true).mode).toBe("hex");
  expect(initialViewerState("a.txt", new Uint8Array([0xff, 0xfe, 0x61, 0]), W, true))
    .toEqual({ mode: "text", encoding: "utf-16le", wrap: false, top: 2, left: 0 }); // il BOM non si mostra
  expect(initialViewerState("a.md", txt, W, true).mode).toBe("text"); // il Markdown renderizzato è dell'ispettore
});

test("initial state: native kinds, archives as text or hex, ANSI fallback and truncated samples", () => {
  const txt = new TextEncoder().encode("ciao");
  for (const [name, mode] of [["a.pdf", "pdf"], ["a.mp4", "video"], ["a.flac", "audio"]] as const) {
    expect(initialViewerState(name, txt, W, true).mode, name).toBe(mode);
  }
  expect(initialViewerState("a.zip", new Uint8Array([0x50, 0x4b, 0, 0]), W, true).mode).toBe("hex");
  expect(initialViewerState("a.txt", new Uint8Array([0xe8]), W, true).encoding).toBe(W); // UTF-8 non valido
  // Un campione di file più grande che finisce a metà di una sequenza UTF-8 resta UTF-8.
  expect(initialViewerState("a.txt", new Uint8Array([0x61, 0xc3]), W, false).encoding).toBe("utf-8");
});

test("alignTop keeps hex rows on 16 bytes and text on the code unit", () => {
  expect(alignTop(35, "hex", "utf-8")).toBe(32);
  expect(alignTop(35, "text", "utf-8")).toBe(35);
  expect(alignTop(35, "text", "utf-16le")).toBe(34);
  expect(alignTop(0, "hex", "utf-16be")).toBe(0);
});

beforeEach(() => useViewers.setState({ states: {} }));

test("the store merges patches per tab and dispose forgets the tab", () => {
  const { set, dispose } = useViewers.getState();
  set("t1", initialViewerState("a.txt", new TextEncoder().encode("x"), W, true));
  set("t1", { top: 40, wrap: true });
  expect(useViewers.getState().states.t1).toEqual({ mode: "text", encoding: "utf-8", wrap: true, top: 40, left: 0 });
  dispose("t1");
  expect(useViewers.getState().states.t1).toBeUndefined();
});

test("a late patch for a closed tab does not bring its state back", () => {
  useViewers.getState().set("gone", { top: 10 });
  expect(useViewers.getState().states.gone).toBeUndefined();
});

test("archive entries are always extracted to a local copy before reading, remote files only for native views", () => {
  const arc = "arc://C:\\a.zip/x.txt";
  expect(needsLocalCopy(arc, "text", false)).toBe(true);
  expect(needsLocalCopy(arc, "hex", false)).toBe(true);
  expect(needsLocalCopy(arc, "image", false)).toBe(true);
  expect(needsLocalCopy(arc, "text", true)).toBe(false); // copia già pronta
  expect(needsLocalCopy(arc, undefined, false)).toBe(false); // stato non ancora creato
  expect(needsLocalCopy("sftp://h/a.txt", "text", false)).toBe(false);
  expect(needsLocalCopy("sftp://h/a.png", "image", false)).toBe(true);
  expect(needsLocalCopy("C:\\a.png", "image", false)).toBe(false);
});

test("the sample is skipped for native views of remote files and archive entries, not for local ones", () => {
  expect(skipsSample("arc://C:\\a.zip/x.png", "image")).toBe(true);
  expect(skipsSample("sftp://h/a.mp4", "video")).toBe(true);
  expect(skipsSample("arc://C:\\a.zip/x.txt", null)).toBe(false);
  expect(skipsSample("C:\\a.png", "image")).toBe(false);
});
