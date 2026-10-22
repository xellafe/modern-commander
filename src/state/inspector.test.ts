import { afterEach, beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
vi.mock("./sizeRequests");
vi.mock("../lib/monaco", async (orig) => ({ ...(await orig<typeof import("../lib/monaco")>()), loadMonaco: vi.fn() }));
import * as api from "../ipc/api";
import type { ArchiveListing, DirSize, Drive, FileInfo } from "../ipc/types";
import { renderMarkdown } from "../lib/markdown";
import { loadMonaco } from "../lib/monaco";
import { ARCHIVE_LIMIT, DEBOUNCE_MS, PREVIEW_BYTES, useInspector } from "./inspector";
import { cancelTabSizes, computeDirSize } from "./sizeRequests";

const fileInfo = vi.mocked(api.fileInfo);
const readChunk = vi.mocked(api.readChunk);
const calcHash = vi.mocked(api.calcHash);
const listArchive = vi.mocked(api.listArchive);
const cancelRequest = vi.mocked(api.cancelRequest);
const dirSize = vi.mocked(computeDirSize);

const INFO: FileInfo = { size: 10, isDir: false, createdMs: 1, modifiedMs: 2, accessedMs: 3, attributes: 0, mtimeReliable: true };
const enc = (s: string) => new TextEncoder().encode(s);
const set = (path: string, isDir = false) => useInspector.getState().setTarget({ path, isDir });
const state = () => useInspector.getState();
const sleep = (ms: number) => vi.advanceTimersByTimeAsync(ms);

beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
  let id = 0;
  fileInfo.mockResolvedValue(INFO);
  cancelRequest.mockResolvedValue();
  vi.mocked(api.nextRequestId).mockImplementation(() => ++id);
  dirSize.mockReturnValue(new Promise(() => {}));
  vi.mocked(loadMonaco).mockResolvedValue({ languages: { getLanguages: () => [{ id: "typescript", extensions: [".ts"] }] } } as never);
  useInspector.setState({ target: null, info: null, preview: null, hashes: {} });
});
afterEach(() => {
  state().setTarget(null); // ferma un debounce ancora armato
  vi.useRealTimers();
});

test("debounces and only the latest target wins", async () => {
  readChunk.mockImplementation((p) => new Promise((r) => setTimeout(() => r(enc(p)), p.endsWith("a.txt") ? 500 : 10)));
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10); // parte la lettura di a (lenta)
  expect(readChunk).toHaveBeenCalledTimes(1);
  set("C:\\b.txt");
  await sleep(1000);
  expect(state().preview).toMatchObject({ kind: "text", text: "C:\\b.txt" });
  expect(state().target?.path).toBe("C:\\b.txt");
});

test("rapid moves inside the debounce read nothing", async () => {
  readChunk.mockImplementation(async (p) => enc(p));
  for (let i = 0; i < 20; i++) {
    set(`C:\\f${i}.txt`);
    await sleep(10);
  }
  expect(readChunk).not.toHaveBeenCalled();
  expect(fileInfo).not.toHaveBeenCalled();
  await sleep(DEBOUNCE_MS);
  expect(readChunk).toHaveBeenCalledTimes(1);
  expect(readChunk).toHaveBeenCalledWith("C:\\f19.txt", 0, PREVIEW_BYTES);
  expect(fileInfo).toHaveBeenCalledTimes(1);
  expect(state().preview).toMatchObject({ kind: "text", text: "C:\\f19.txt" });
});

test("changing target cancels folder size and hash", async () => {
  set("C:\\dir", true);
  await sleep(DEBOUNCE_MS + 10);
  expect(dirSize).toHaveBeenCalledWith("inspector", "C:\\dir");
  expect(state().preview).toEqual({ kind: "folder", size: null });

  readChunk.mockResolvedValue(enc("x"));
  vi.mocked(cancelTabSizes).mockClear();
  set("C:\\a.txt");
  expect(cancelTabSizes).toHaveBeenCalledWith("inspector"); // subito, senza attendere il debounce
  await sleep(DEBOUNCE_MS + 10);

  calcHash.mockReturnValue(new Promise(() => {}));
  state().requestHash("sha256");
  const id = calcHash.mock.calls[0][0];
  expect(calcHash).toHaveBeenCalledWith(id, "C:\\a.txt", "sha256");
  expect(state().hashes).toEqual({ sha256: "pending" });

  cancelRequest.mockClear();
  set("C:\\b.txt");
  expect(cancelRequest).toHaveBeenCalledWith(id); // subito, senza attendere il debounce
  await sleep(DEBOUNCE_MS + 10);
  expect(state().hashes).toEqual({});
});

test("a hash that finishes after a target change is dropped, one that finishes in time is shown", async () => {
  readChunk.mockResolvedValue(enc("x"));
  let finish!: (h: string) => void;
  calcHash.mockImplementationOnce(() => new Promise((r) => (finish = r)));
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10);
  state().requestHash("md5");
  set("C:\\b.txt");
  await sleep(DEBOUNCE_MS + 10);
  finish("old");
  await sleep(0);
  expect(state().hashes).toEqual({});

  calcHash.mockResolvedValueOnce("abc");
  state().requestHash("sha1");
  await sleep(0);
  expect(state().hashes).toEqual({ sha1: "abc" });
});

test("hiding the inspector (null target) cancels the work and a late result does not revive the preview", async () => {
  let read!: (b: Uint8Array<ArrayBuffer>) => void;
  readChunk.mockImplementation(() => new Promise((r) => (read = r)));
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10);
  calcHash.mockReturnValue(new Promise(() => {}));
  state().requestHash("md5");
  const id = calcHash.mock.calls[0][0];
  vi.mocked(cancelTabSizes).mockClear();
  cancelRequest.mockClear();
  state().setTarget(null);
  expect(cancelTabSizes).toHaveBeenCalledWith("inspector");
  expect(cancelRequest).toHaveBeenCalledWith(id);
  read(enc("late"));
  await sleep(1000);
  expect(state()).toMatchObject({ target: null, info: null, preview: null, hashes: {} });
});

test("binary files are not shown as text", async () => {
  readChunk.mockResolvedValue(new Uint8Array([0x00, 0x01]));
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "binary" });
});

test("an empty file and a BOM-only file are empty text, not binary", async () => {
  await renderMarkdown(""); // import dinamico = I/O reale, vedi il test del markdown
  readChunk.mockResolvedValue(new Uint8Array());
  set("C:\\empty.txt");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "text", text: "", language: null, truncated: false });

  readChunk.mockResolvedValue(new Uint8Array([0xef, 0xbb, 0xbf]));
  set("C:\\bom.txt");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "text", text: "", language: null, truncated: false });

  readChunk.mockResolvedValue(new Uint8Array());
  set("C:\\empty.md");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "markdown", html: "", truncated: false });
});

test("text: truncated flag from the real size, language from Monaco, UTF-16 decoded", async () => {
  fileInfo.mockResolvedValue({ ...INFO, size: PREVIEW_BYTES + 1 });
  readChunk.mockResolvedValue(enc("let a"));
  set("C:\\a.ts");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "text", text: "let a", language: "typescript", truncated: true });
  expect(state().info?.size).toBe(PREVIEW_BYTES + 1);

  fileInfo.mockResolvedValue(INFO);
  readChunk.mockResolvedValue(new Uint8Array([0xff, 0xfe, 0x68, 0x00, 0x69, 0x00]));
  set("C:\\b.txt");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "text", text: "hi", language: null, truncated: false });
});

test("markdown is rendered without raw html; image, pdf, video read nothing", async () => {
  await renderMarkdown(""); // l'import dinamico di markdown-it è I/O reale: i timer finti non lo aspettano
  readChunk.mockResolvedValue(enc("# T\n<script>x</script>"));
  set("C:\\a.md");
  await sleep(DEBOUNCE_MS + 10);
  const p = state().preview;
  expect(p).toMatchObject({ kind: "markdown", truncated: false });
  expect(p && "html" in p && p.html).toContain("<h1>");
  expect(p && "html" in p && p.html).not.toContain("<script");

  readChunk.mockClear();
  for (const [name, kind] of [["a.png", "image"], ["a.pdf", "pdf"], ["a.mp4", "video"], ["a.mp3", "audio"]] as const) {
    set(`C:\\${name}`);
    await sleep(DEBOUNCE_MS + 10);
    expect(state().preview).toEqual({ kind, path: `C:\\${name}` });
  }
  expect(readChunk).not.toHaveBeenCalled();
});

test("remote file: metadata and text preview only, no media or archive", async () => {
  for (const name of ["a.png", "a.pdf", "a.mp4", "a.zip", "a.7z"]) {
    set(`sftp://h/${name}`);
    await sleep(DEBOUNCE_MS + 10);
    expect(state()).toMatchObject({ info: INFO, preview: { kind: "none" } });
  }
  expect(readChunk).not.toHaveBeenCalled();
  expect(listArchive).not.toHaveBeenCalled();
  readChunk.mockResolvedValue(enc("ciao"));
  set("sftp://h/a.txt");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toMatchObject({ kind: "text", text: "ciao" });
});

test("folder: size arrives, but not if the target changed meanwhile", async () => {
  const size: DirSize = { bytes: 5, files: 2, dirs: 1, skipped: 0 };
  let done!: (s: DirSize) => void;
  dirSize.mockImplementationOnce(() => new Promise((r) => (done = r)));
  set("C:\\dir", true);
  await sleep(DEBOUNCE_MS + 10);
  done(size);
  await sleep(0);
  expect(state().preview).toEqual({ kind: "folder", size });

  let late!: (s: DirSize) => void;
  dirSize.mockImplementationOnce(() => new Promise((r) => (late = r)));
  set("C:\\dir2", true);
  await sleep(DEBOUNCE_MS + 10);
  readChunk.mockResolvedValue(enc("x"));
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10);
  late(size);
  await sleep(0);
  expect(state().preview).toMatchObject({ kind: "text" });
});

test("drive row shows the drive and reads nothing", async () => {
  const drive: Drive = { root: "C:\\", label: "", kind: "fixed", total: 2, free: 1, ready: true };
  state().setTarget({ path: "C:\\", isDir: true, drive });
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "drive", drive });
  expect(fileInfo).not.toHaveBeenCalled();
  expect(dirSize).not.toHaveBeenCalled();
});

test("archive: first ARCHIVE_LIMIT entries; the listing is cancelled by a target change", async () => {
  const listing: ArchiveListing = { entries: [], total: 3, truncated: false };
  listArchive.mockResolvedValueOnce(listing);
  set("C:\\a.zip");
  await sleep(DEBOUNCE_MS + 10);
  expect(listArchive).toHaveBeenCalledWith(expect.any(Number), "C:\\a.zip", ARCHIVE_LIMIT);
  expect(state().preview).toEqual({ kind: "archive", listing });

  listArchive.mockReturnValueOnce(new Promise(() => {}));
  set("C:\\b.zip");
  await sleep(DEBOUNCE_MS + 10);
  const id = listArchive.mock.calls[1][0];
  cancelRequest.mockClear();
  set("C:\\c.txt");
  expect(cancelRequest).toHaveBeenCalledWith(id);
});

test("7z: no automatic listing, only the explicit request lists it", async () => {
  set("C:\\a.7z");
  await sleep(DEBOUNCE_MS + 10);
  expect(listArchive).not.toHaveBeenCalled();
  expect(state().preview).toEqual({ kind: "archiveOnDemand" });
  expect(state().info).toEqual(INFO); // i metadati ci sono comunque

  const listing: ArchiveListing = { entries: [], total: 2, truncated: false };
  listArchive.mockResolvedValueOnce(listing);
  state().requestArchive();
  expect(state().preview).toEqual({ kind: "loading" });
  await sleep(0);
  expect(listArchive).toHaveBeenCalledWith(expect.any(Number), "C:\\a.7z", ARCHIVE_LIMIT);
  expect(state().preview).toEqual({ kind: "archive", listing });
});

test("7z: the explicit listing is cancelled by a target change and its result dropped", async () => {
  set("C:\\a.7z");
  await sleep(DEBOUNCE_MS + 10);
  let done!: (l: ArchiveListing) => void;
  listArchive.mockReturnValueOnce(new Promise((r) => (done = r)));
  state().requestArchive();
  await sleep(0);
  const id = listArchive.mock.calls[0][0];
  cancelRequest.mockClear();
  set("C:\\b.txt");
  expect(cancelRequest).toHaveBeenCalledWith(id);
  done({ entries: [], total: 1, truncated: false });
  await sleep(0);
  expect(state().preview).not.toMatchObject({ kind: "archive" }); // niente risultato del vecchio bersaglio
});

test("requestArchive does nothing for a non-7z target or while the preview is not the on-demand one", async () => {
  readChunk.mockResolvedValue(enc("x"));
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10);
  state().requestArchive();
  expect(listArchive).not.toHaveBeenCalled();
  expect(state().preview).toMatchObject({ kind: "text" });
});

test("a failing read shows the error code", async () => {
  readChunk.mockRejectedValue({ code: "ACCESS_DENIED", path: "C:\\a.txt", detail: null });
  set("C:\\a.txt");
  await sleep(DEBOUNCE_MS + 10);
  expect(state().preview).toEqual({ kind: "error", code: "ACCESS_DENIED" });
});
