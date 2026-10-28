// Comandi sugli archivi (fase 3b, Task 10): apri come cartella, crea uno ZIP, estrai.
import { fileInfo, listDir } from "../ipc/api";
import { dirTarget, folderName, formatArchive, isArchive, isVirtual, joinPath, sourceCaps } from "../lib/paths";
import { useLayout } from "../state/layout";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { useUi, type DialogRequest } from "../state/ui";
import { copyTargets, enqueue, selection } from "./fileOps";
import { registerCommand, type Command } from "./registry";

const views = () => useTabViews.getState();

/** L'archivio sotto il cursore (o il solo marcato) con la sua radice; `null` se l'elemento non è un archivio apribile. */
function archiveUnderCursor() {
  const { tabId, path, markedOrCursor } = activeTabContext();
  const [row, ...rest] = markedOrCursor;
  const root = row && !rest.length && !row.isDir ? dirTarget(path, row) : null;
  return root && isArchive(root) ? { tabId, path, name: row.name, root } : null;
}

/** Nome della cartella in cui estrarre: quello dell'archivio senza estensione (`a.tar.gz` → `a`). */
const unpackFolder = (name: string) => name.replace(/\.(tar\.gz|tgz|[^.]+)$/i, "") || name;

/** Nome del file di uscita: `.zip` imposto se manca. */
export const zipName = (name: string) => (/\.zip$/i.test(name.trim()) ? name.trim() : `${name.trim()}.zip`);

function openArchive() {
  const a = archiveUnderCursor();
  if (a) return views().load(a.tabId, a.root);
}

function openPack() {
  const sel = selection();
  if (!sel) return;
  const destDir = copyTargets(useLayout.getState()).dest;
  // Non si crea uno ZIP dentro un archivio (niente annidamento), né dove non si può scrivere.
  if (isArchive(sel.path) || isArchive(destDir) || !sourceCaps(destDir).write) {
    views().showNotice(sel.tabId, { code: "READ_ONLY_SOURCE" });
    return;
  }
  const base = sel.sources.length === 1 ? folderName(sel.sources[0]) : folderName(sel.path);
  useUi.getState().openDialog({ kind: "pack", sources: sel.sources, name: zipName(base), destDir });
}

/**
 * OK del dialogo Comprimi. `"exists"`: il file esiste già e `replace` non c'è, il dialogo chiede se sovrascrivere.
 * `"stale"`: `isCurrent` è diventata falsa durante il controllo (Annulla, nome o cartella cambiati) e il job non parte.
 * ponytail: controllo e job non sono atomici; se il file compare in mezzo, il backend risponde ALREADY_EXISTS.
 */
export async function packSubmit(
  req: Extract<DialogRequest, { kind: "pack" }>,
  name: string,
  destDir: string,
  replace: boolean,
  isCurrent: () => boolean = () => true,
): Promise<"queued" | "exists" | "stale"> {
  const file = joinPath(destDir, zipName(name));
  const exists = !replace && (await fileInfo(file).then(() => true, () => false));
  if (!isCurrent()) return "stale";
  if (exists) return "exists";
  // `dest` è la radice dello ZIP da creare.
  await enqueue({ kind: { type: "pack", replace }, sources: req.sources, dest: formatArchive(file) });
  return "queued";
}

async function unpack() {
  const a = archiveUnderCursor();
  if (!a) return;
  // Gruppo di destinazione e cartella di partenza presi subito: durante l'attesa l'utente può cambiare scheda o gruppo.
  const t = copyTargets(useLayout.getState());
  let entries;
  try {
    entries = await listDir(a.root);
  } catch (e) {
    views().showNotice(a.tabId, e);
    return;
  }
  if (!entries.length) {
    views().showNotice(a.tabId, { code: "EMPTY_ARCHIVE" });
    return;
  }
  // Il separatore finale dice al backend che la destinazione è una cartella, anche con una sola voce.
  const dest = joinPath(t.dest, unpackFolder(a.name)) + (isVirtual(t.dest) ? "/" : "\\");
  // La radice di un archivio non è mai una sorgente: lo sono le sue voci.
  useUi.getState().openDialog({
    kind: "copyMove", move: false, sources: entries.map((e) => joinPath(a.root, e.name)), from: a.path,
    dest: sourceCaps(t.dest).write ? dest : "",
    requireChange: false, targets: t.targets, policy: "ask",
  });
}

const DEFS: [string, Command["run"]][] = [
  ["mc_OpenArchive", openArchive],
  ["cm_PackFiles", openPack],
  ["cm_UnpackFiles", unpack],
];

export const ARCHIVE_COMMAND_IDS = DEFS.map(([id]) => id);

export function registerArchiveCommands() {
  for (const [id, run] of DEFS) registerCommand({ id, titleKey: `commands.${id}`, when: "fileList", run });
}
