import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../ipc/api");
import * as api from "../ipc/api";
import type { Entry } from "../ipc/types";
import { initialLayout, useLayout } from "../state/layout";
import { useTabViews } from "../state/tabViews";
import { useUi } from "../state/ui";
import { ARCHIVE_COMMAND_IDS, packSubmit, registerArchiveCommands, toggleUnpackFolder } from "./archives";
import { executeCommand } from "./registry";
import { DEFAULT_KEYMAP } from "./tcKeymap";

const f = (name: string, isDir = false): Entry => ({ name, isDir, size: 0, modifiedMs: 0, createdMs: 0, accessedMs: 0, attributes: 0, isSymlink: false });

// Gruppo attivo su `left` (listing `rows`, cursore sulla prima riga vera), l'altro su `right`.
async function setup(left: string, right: string, rows: Entry[]) {
  vi.resetAllMocks();
  vi.mocked(api.watchDir).mockResolvedValue();
  vi.mocked(api.unwatchDir).mockResolvedValue();
  const l = initialLayout([left, right]);
  useLayout.setState(l);
  useTabViews.setState({ views: {}, notice: null });
  useUi.setState({ dialog: null });
  const t1 = l.groups[0].activeTabId;
  vi.mocked(api.listDir).mockResolvedValueOnce(rows);
  await useTabViews.getState().load(t1, left);
  useTabViews.getState().update(t1, (v) => ({ ...v, cursor: 1 }));
  return t1;
}

beforeEach(registerArchiveCommands);

test("every archive command has a title in both languages", async () => {
  const en = (await import("../i18n/en.json")).default as { commands: Record<string, string> };
  const it = (await import("../i18n/it.json")).default as { commands: Record<string, string> };
  for (const id of ARCHIVE_COMMAND_IDS) {
    expect(en.commands[id], id).toBeTruthy();
    expect(it.commands[id], id).toBeTruthy();
  }
  for (const k of ["EMPTY_ARCHIVE"]) {
    expect((en as never as { errors: Record<string, string> }).errors[k]).toBeTruthy();
    expect((it as never as { errors: Record<string, string> }).errors[k]).toBeTruthy();
  }
});

test("keymap binds the TC keys", () => {
  expect(DEFAULT_KEYMAP["Alt+F5"]).toBe("cm_PackFiles");
  expect(DEFAULT_KEYMAP["Alt+F9"]).toBe("cm_UnpackFiles");
  expect(DEFAULT_KEYMAP["Ctrl+PageDown"]).toBe("mc_OpenArchive");
});

test("mc_OpenArchive opens the archive under the cursor and ignores anything else", async () => {
  const t1 = await setup("C:\\w", "D:\\out", [f("a.zip"), f("n.txt")]);
  vi.mocked(api.listDir).mockResolvedValueOnce([]);
  await executeCommand("mc_OpenArchive");
  expect(useLayout.getState().tabs[t1].path).toBe("arc://C:\\w\\a.zip/");
  await setup("C:\\w", "D:\\out", [f("a.zip"), f("n.txt")]);
  useTabViews.getState().update(useLayout.getState().groups[0].activeTabId, (v) => ({ ...v, cursor: 2 }));
  await executeCommand("mc_OpenArchive");
  expect(useLayout.getState().tabs[useLayout.getState().groups[0].activeTabId].path).toBe("C:\\w");
});

test("pack proposes name and other group", async () => {
  const t1 = await setup("C:\\w", "D:\\out", [f("src", true), f("b.txt")]);
  useTabViews.getState().update(t1, (v) => ({ ...v, marked: new Set(["src"]) }));
  await executeCommand("cm_PackFiles");
  expect(useUi.getState().dialog).toMatchObject({ kind: "pack", sources: ["C:\\w\\src"], name: "src.zip", destDir: "D:\\out" });
  // Più elementi: il nome della cartella corrente.
  await setup("C:\\w", "D:\\out", [f("a.txt"), f("b.txt")]);
  useTabViews.getState().update(useLayout.getState().groups[0].activeTabId, (v) => ({ ...v, marked: new Set(["a.txt", "b.txt"]) }));
  await executeCommand("cm_PackFiles");
  expect(useUi.getState().dialog).toMatchObject({ kind: "pack", name: "w.zip" });
});

test("pack is refused inside an archive and into a destination that is not writable", async () => {
  await setup("arc://C:\\w\\a.zip/", "D:\\out", [f("x")]);
  await executeCommand("cm_PackFiles");
  expect(useUi.getState().dialog).toBeNull();
  expect(useTabViews.getState().notice?.code).toBe("READ_ONLY_SOURCE");
  await setup("C:\\w", "http://h/", [f("x")]);
  await executeCommand("cm_PackFiles");
  expect(useUi.getState().dialog).toBeNull();
  expect(useTabViews.getState().notice?.code).toBe("READ_ONLY_SOURCE");
});

test("pack submits a pack job", async () => {
  vi.mocked(api.enqueueJob).mockResolvedValue(1);
  const req = { kind: "pack" as const, sources: ["C:\\w\\src"], name: "src.zip", destDir: "D:\\out" };
  vi.mocked(api.fileInfo).mockRejectedValue({ code: "NOT_FOUND" });
  expect(await packSubmit(req, "src.zip", "D:\\out", false)).toBe("queued");
  expect(vi.mocked(api.enqueueJob).mock.lastCall?.[0]).toMatchObject({
    kind: { type: "pack", replace: false }, sources: ["C:\\w\\src"], dest: "arc://D:\\out\\src.zip/",
  });
  // Senza `.zip` nel nome lo si aggiunge.
  await packSubmit(req, "altro", "D:\\out", false);
  expect(vi.mocked(api.enqueueJob).mock.lastCall?.[0].dest).toBe("arc://D:\\out\\altro.zip/");
  // File esistente: avviso (niente job), poi "Sovrascrivi".
  vi.mocked(api.enqueueJob).mockClear();
  vi.mocked(api.fileInfo).mockResolvedValue({ isDir: false } as never);
  expect(await packSubmit(req, "src.zip", "D:\\out", false)).toBe("exists");
  expect(api.enqueueJob).not.toHaveBeenCalled();
  expect(await packSubmit(req, "src.zip", "D:\\out", true)).toBe("queued");
  expect(vi.mocked(api.enqueueJob).mock.lastCall?.[0]).toMatchObject({ kind: { type: "pack", replace: true } });
});

test("pack does not enqueue when the dialog was cancelled or edited during the check", async () => {
  const req = { kind: "pack" as const, sources: ["C:\\w\\src"], name: "src.zip", destDir: "D:\\out" };
  vi.mocked(api.enqueueJob).mockReset().mockResolvedValue(1);
  let current = true;
  vi.mocked(api.fileInfo).mockImplementation(async () => {
    current = false; // Annulla (o una modifica) mentre il server risponde
    throw { code: "NOT_FOUND" };
  });
  expect(await packSubmit(req, "src.zip", "D:\\out", false, () => current)).toBe("stale");
  // Anche con un file che esiste: il risultato di un controllo vecchio non si mostra.
  current = true;
  vi.mocked(api.fileInfo).mockImplementation(async () => {
    current = false;
    return { isDir: false } as never;
  });
  expect(await packSubmit(req, "src.zip", "D:\\out", false, () => current)).toBe("stale");
  // Sovrascrivi già cliccato, poi Annulla prima dell'invio: niente job.
  expect(await packSubmit(req, "src.zip", "D:\\out", true, () => false)).toBe("stale");
  expect(api.enqueueJob).not.toHaveBeenCalled();
});

test("unpack lists the root into the copy dialog", async () => {
  await setup("C:\\w", "D:\\out", [f("a.zip"), f("n.txt")]);
  vi.mocked(api.listDir).mockResolvedValueOnce([f("a"), f("b", true)]);
  await executeCommand("cm_UnpackFiles");
  expect(api.listDir).toHaveBeenLastCalledWith("arc://C:\\w\\a.zip/");
  expect(useUi.getState().dialog).toMatchObject({
    kind: "copyMove", move: false, sources: ["arc://C:\\w\\a.zip/a", "arc://C:\\w\\a.zip/b"], dest: "D:\\out\\a\\",
  });
});

test("unpack of an empty archive shows a notice, of a non-archive does nothing", async () => {
  await setup("C:\\w", "D:\\out", [f("a.zip"), f("n.txt")]);
  vi.mocked(api.listDir).mockResolvedValueOnce([]);
  await executeCommand("cm_UnpackFiles");
  expect(useUi.getState().dialog).toBeNull();
  expect(useTabViews.getState().notice?.code).toBe("EMPTY_ARCHIVE");
  const t1 = await setup("C:\\w", "D:\\out", [f("a.zip"), f("n.txt")]);
  useTabViews.getState().update(t1, (v) => ({ ...v, cursor: 2 }));
  await executeCommand("cm_UnpackFiles");
  expect(api.listDir).toHaveBeenCalledTimes(1); // solo il caricamento della scheda
  expect(useUi.getState().dialog).toBeNull();
});

test("the unpack subfolder checkbox adds or removes the archive name from the destination", () => {
  expect(toggleUnpackFolder("D:\\out\\a\\", "a", false)).toBe("D:\\out\\");
  expect(toggleUnpackFolder("D:\\out\\", "a", true)).toBe("D:\\out\\a\\");
  expect(toggleUnpackFolder("D:\\out", "a", true)).toBe("D:\\out\\a\\");
  // Già nello stato chiesto, o con una destinazione scritta a mano che non finisce col nome: invariata.
  expect(toggleUnpackFolder("D:\\out\\a\\", "a", true)).toBe("D:\\out\\a\\");
  expect(toggleUnpackFolder("D:\\altro\\", "a", false)).toBe("D:\\altro\\");
  expect(toggleUnpackFolder("D:\\out\\A\\", "a", false)).toBe("D:\\out\\"); // Windows: maiuscole indifferenti
  expect(toggleUnpackFolder("sftp://w/rel/a/", "a", false)).toBe("sftp://w/rel/");
  expect(toggleUnpackFolder("sftp://w/rel/A/", "a", false)).toBe("sftp://w/rel/A/"); // POSIX: distinte
  expect(toggleUnpackFolder("sftp://w/rel", "a", true)).toBe("sftp://w/rel/a/");
});

test("unpack offers the subfolder option, on by default", async () => {
  await setup("C:\\w", "D:\\out", [f("a.zip"), f("n.txt")]);
  vi.mocked(api.listDir).mockResolvedValueOnce([f("a")]);
  await executeCommand("cm_UnpackFiles");
  expect(useUi.getState().dialog).toMatchObject({ kind: "copyMove", dest: "D:\\out\\a\\", unpackFolder: "a" });
});

test("unpack inside an archive extracts the whole current archive", async () => {
  await setup("arc://C:\\w\\a.zip/conf", "D:\\out", [f("x.xml"), f("b.zip")]);
  vi.mocked(api.listDir).mockResolvedValueOnce([f("conf", true), f("n.txt")]);
  await executeCommand("cm_UnpackFiles");
  expect(api.listDir).toHaveBeenLastCalledWith("arc://C:\\w\\a.zip/");
  expect(useUi.getState().dialog).toMatchObject({
    kind: "copyMove", move: false, sources: ["arc://C:\\w\\a.zip/conf", "arc://C:\\w\\a.zip/n.txt"], dest: "D:\\out\\a\\",
  });
});

test("unpack from an archive on a server keeps the encoded container in the sources", async () => {
  await setup("sftp://w/rel", "D:\\out", [f("a.zip"), f("n.txt")]);
  vi.mocked(api.listDir).mockResolvedValueOnce([f("a"), f("b")]);
  await executeCommand("cm_UnpackFiles");
  expect(api.listDir).toHaveBeenLastCalledWith("arc://sftp:%2F%2Fw%2Frel%2Fa.zip/");
  expect(useUi.getState().dialog).toMatchObject({
    kind: "copyMove", from: "sftp://w/rel", dest: "D:\\out\\a\\",
    sources: ["arc://sftp:%2F%2Fw%2Frel%2Fa.zip/a", "arc://sftp:%2F%2Fw%2Frel%2Fa.zip/b"],
  });
});
