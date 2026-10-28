// Comandi sui file (Task 16): copia, spostamento, rinomina, nuova cartella, eliminazione, selezione per maschera.
import { enqueueJob, fileInfo, normalizePaths, shiftPressed } from "../ipc/api";
import type { ConflictPolicy, JobId, JobSpec } from "../ipc/types";
import { folderName, formatArchive, isArchive, isVirtual, joinPath, normalizePath, parseArchive, parseRemote, sourceCaps } from "../lib/paths";
import { useJobs, waitForJob } from "../state/jobs";
import { targetGroup, useLayout, type LayoutState } from "../state/layout";
import { useSettings } from "../state/settings";
import { parentPath, selectByMask } from "../state/tabView";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { useUi, type CopyTarget, type DialogRequest } from "../state/ui";
import { registerCommand, type Command } from "./registry";
import { dropAction } from "./shell";

/** Destinazione proposta per F5/F6 e, con 3 o più gruppi, gli altri gruppi tra cui scegliere (1…9). */
export function copyTargets(l: LayoutState): { dest: string; requireChange: boolean; targets: CopyTarget[] } {
  const pathOf = (groupId: string) => l.tabs[l.groups.find((g) => g.id === groupId)!.activeTabId].path;
  const target = targetGroup(l);
  if (!target) return { dest: pathOf(l.activeGroupId), requireChange: true, targets: [] };
  const targets = l.groups.length < 3 ? [] : l.groups.flatMap((g, i) =>
    g.id === l.activeGroupId ? [] : [{ n: i + 1, groupId: g.id, path: pathOf(g.id) }]);
  return { dest: pathOf(target), requireChange: false, targets };
}

const INVALID_CHARS = /[<>:"/\\|?*\x00-\x1f]/;

/** Chiave i18n dell'errore di un nome di file o cartella dentro `base`, `null` se è valido. */
export function nameError(name: string, base = ""): string | null {
  if (!name.trim()) return "ops.nameEmpty";
  // Sul server e nell'archivio vale POSIX: solo `/` e NUL, più `.` e `..` che il backend risolverebbe come percorso.
  if (isVirtual(base)) return /[/\0]/.test(name) || name === "." || name === ".." ? "ops.nameInvalid" : null;
  // Windows toglie in silenzio punti e spazi finali: meglio rifiutarli che creare un nome diverso.
  return INVALID_CHARS.test(name) || /[. ]$/.test(name) ? "ops.nameInvalid" : null;
}

const ABS_ROOT = /^(?:[A-Za-z]:\\|\\\\[^\\/]+\\[^\\/]+\\)/;

/** Su una scheda remota un `/` iniziale è la radice del server (`/etc` → `sftp://w/etc`), in un archivio la sua radice. */
function serverAbsolute(base: string, input: string): string {
  if (!input.startsWith("/")) return input;
  const r = parseRemote(base);
  const a = parseArchive(base);
  return r ? `${r.scheme}://${r.conn}${input}` : a ? formatArchive(a.container, input) : input;
}

/**
 * Percorso da creare con F7: relativo alla cartella corrente (con `\`, o `/` sul remoto, per le intermedie) o assoluto.
 * `focus` è il nome, dentro la cartella corrente, su cui portare il cursore dopo la creazione.
 */
export function mkdirTarget(base: string, input: string): { path: string; focus: string | null } | { error: string } {
  const s = serverAbsolute(base, input.trim());
  const r = parseRemote(s);
  const a = parseArchive(s);
  const root = r ? `${r.scheme}://${r.conn}/` : a ? formatArchive(a.container) : (ABS_ROOT.exec(s)?.[0] ?? "");
  if (!root && !sourceCaps(base).write) return { error: "ops.nameInvalid" };
  const where = root || base;
  const sep = isVirtual(where) ? "/" : "\\";
  const parts = (r ? r.path : a ? a.inner : s.slice(root.length)).split(sep).filter(Boolean);
  if (!parts.length) return { error: "ops.nameEmpty" };
  for (const p of parts) {
    const e = nameError(p, where);
    if (e) return { error: e };
  }
  const rel = parts.join(sep);
  if (!root) return { path: joinPath(base, rel), focus: parts[0] };
  const path = root + rel;
  // Sul server e nell'archivio i nomi distinguono maiuscole e minuscole.
  const same = r || a ? parentPath(path) === base : parentPath(path)?.toLowerCase() === base.toLowerCase();
  return { path, focus: same ? parts[parts.length - 1] : null };
}

/**
 * Destinazione digitata nella finestra di copia: assoluta, oppure relativa alla cartella delle sorgenti.
 * Un separatore finale resta: con un solo file da copiare dice che la destinazione è una cartella
 * e non il nome del file (regola di TC; sul remoto lo applica `startCopyMove`).
 */
export function resolveDest(base: string, input: string): string | null {
  const s = input.trim();
  if (!s) return null;
  const p = normalizePath(serverAbsolute(base, s));
  const abs = /^(?:[A-Za-z]:\\|\\\\)/.test(p) || isVirtual(p) ? p
    : !sourceCaps(base).write ? null
    : isVirtual(base) ? normalizePath(joinPath(base, s)) : joinPath(base, p);
  const sep = abs && isVirtual(abs) ? "/" : "\\";
  return abs && /[\\/]$/.test(s) && !abs.endsWith(sep) ? abs + sep : abs;
}

// Sul server e nell'archivio i nomi distinguono maiuscole e minuscole.
const noTrailing = (p: string) => (isVirtual(p) ? p.replace(/\/+$/, "") : p.replace(/\\+$/, "").toLowerCase());

/** OK disattivato: destinazione uguale alla cartella delle sorgenti con un solo gruppo o per uno spostamento. */
export const mustChangeDest = (req: { from: string; move: boolean; requireChange: boolean }, dest: string | null) =>
  (req.requireChange || req.move) && dest !== null && noTrailing(dest) === noTrailing(req.from);

const views = () => useTabViews.getState();
const ui = () => useUi.getState();

const enqueue = (spec: Omit<JobSpec, "filter" | "policy"> & Partial<JobSpec>) =>
  enqueueJob({ filter: null, policy: { type: "ask" }, ...spec });

/** Dopo un job riuscito porta il cursore su `name`, se la scheda è ancora in quella cartella. */
async function focusWhenDone(id: JobId, tabId: string, base: string, name: string) {
  const job = await waitForJob(id);
  if (job.state.type !== "done" || useLayout.getState().tabs[tabId]?.path !== base) return;
  await views().load(tabId, base, { focus: name });
}

/** Contesto dei comandi sulle selezioni; `null` nella lista delle unità o senza elementi. */
function selection() {
  const ctx = activeTabContext();
  const sources = ctx.markedOrCursor.flatMap((r) => (r.drive ? [] : [joinPath(ctx.path, r.name)]));
  return sources.length ? { ...ctx, sources } : null;
}

/** Sorgente in sola lettura: su HTTP la notifica al posto del backend, nella lista delle unità niente (come prima). */
function readOnly(tabId: string, path: string): boolean {
  if (sourceCaps(path).write) return false;
  if (isVirtual(path)) views().showNotice(tabId, { code: "READ_ONLY_SOURCE" });
  return true;
}

function openCopyMove(move: boolean, samePanel: boolean) {
  const sel = selection();
  if (!sel || (move && readOnly(sel.tabId, sel.path))) return;
  // Shift+F5: copia nella stessa cartella, con un nome nuovo (o il prompt di conflitto se si sceglie "Chiedi").
  const t = samePanel ? { dest: sel.path, requireChange: false, targets: [] } : copyTargets(useLayout.getState());
  ui().openDialog({
    kind: "copyMove", move, sources: sel.sources, from: sel.path,
    dest: sourceCaps(t.dest).write ? t.dest : "", requireChange: t.requireChange, targets: t.targets,
    policy: samePanel ? "renameNew" : "ask",
  });
}

export async function startCopyMove(
  req: Extract<DialogRequest, { kind: "copyMove" }>,
  dest: string,
  filter: string,
  policy: ConflictPolicy["type"],
) {
  // `Loc` toglie il `/` finale di un indirizzo remoto: per un file solo verso `dir/` il nome lo mette il frontend.
  const [src] = req.sources;
  if (req.sources.length === 1 && isVirtual(dest) && dest.endsWith("/") && (await fileInfo(src).then((i) => !i.isDir, () => false))) {
    dest = joinPath(dest, folderName(src));
  }
  return enqueue({
    kind: { type: req.move ? "move" : "copy" }, sources: req.sources, dest,
    filter: filter.trim() || null, policy: { type: policy },
  });
}

/**
 * Drop su un gruppo, da Explorer o di righe trascinate da un altro gruppo: finestra di copia verso la
 * cartella della sua scheda attiva. Le righe rilasciate sul loro stesso gruppo si ignorano.
 */
export async function dropOnGroup(groupId: string, paths: string[], move?: boolean) {
  const l = useLayout.getState();
  const g = l.groups.find((x) => x.id === groupId);
  // Con una finestra o un prompt aperti il drop si ignora: non si perde ciò che l'utente sta scrivendo.
  if (!g || !paths.length || ui().dialog || useJobs.getState().prompts.length) return;
  const action = dropAction(paths, groupId);
  if (action.kind === "ignore") return;
  // Maiusc al rilascio sposta, come in TC e in Explorer. L'evento di drop di Tauri non porta i modificatori: per i
  // drop nativi lo stato del tasto si chiede al sistema subito dopo il rilascio, prima di ogni altra attesa.
  move ??= await shiftPressed();
  // normalizePaths riguarda i percorsi di Explorer: un indirizzo remoto (drag tra i gruppi dell'app) non ci passa.
  const sources = paths.some(isVirtual) ? paths : await normalizePaths(paths);
  const dest = l.tabs[g.activeTabId].path;
  ui().openDialog({
    kind: "copyMove", move, sources,
    from: action.kind === "fromGroup" ? action.from : (parentPath(sources[0]) ?? sources[0]),
    dest: sourceCaps(dest).write ? dest : "", requireChange: false, targets: [], policy: "ask",
  });
}

export async function createFolder(tabId: string, base: string, input: string) {
  const t = mkdirTarget(base, input);
  if ("error" in t) return;
  const id = await enqueue({ kind: { type: "mkdir" }, sources: [t.path], dest: null });
  if (t.focus) await focusWhenDone(id, tabId, base, t.focus);
}

export async function renameInPlace(tabId: string, base: string, oldName: string, newName: string) {
  views().setOverlay(null);
  if (newName === oldName || nameError(newName, base)) return;
  const id = await enqueue({ kind: { type: "rename" }, sources: [joinPath(base, oldName)], dest: joinPath(base, newName) });
  await focusWhenDone(id, tabId, base, newName);
}

export const deleteSources = (sources: string[], permanent: boolean) =>
  enqueue({ kind: { type: "delete", permanent }, sources, dest: null });

async function openDelete(permanent: boolean) {
  const sel = selection();
  if (!sel || readOnly(sel.tabId, sel.path)) return;
  // Dentro un archivio non c'è il Cestino: si elimina sempre in modo definitivo (e la conferma lo dice).
  permanent ||= isArchive(sel.path);
  if (useSettings.getState().confirmDelete) ui().openDialog({ kind: "delete", permanent, sources: sel.sources });
  else await deleteSources(sel.sources, permanent);
}

export const applyMask = (tabId: string, mask: string, select: boolean) =>
  views().update(tabId, (v) => selectByMask(v, mask, select));

function startRename() {
  const { tabId, path, cursorRow } = activeTabContext();
  if (!cursorRow || cursorRow.isParent || cursorRow.drive || readOnly(tabId, path)) return;
  views().setOverlay({ kind: "rename", tabId, name: cursorRow.name });
}

function openMkdir() {
  const { tabId, path } = activeTabContext();
  if (!readOnly(tabId, path)) ui().openDialog({ kind: "mkdir", tabId, path });
}

const DEFS: [string, Command["run"]][] = [
  ["cm_Copy", () => openCopyMove(false, false)],
  ["cm_CopySamepanel", () => openCopyMove(false, true)],
  ["cm_RenMov", () => openCopyMove(true, false)],
  ["cm_RenameOnly", startRename],
  ["cm_MkDir", openMkdir],
  ["cm_Delete", () => openDelete(false)],
  ["mc_DeletePermanent", () => openDelete(true)],
  ["cm_SpreadSelection", () => ui().openDialog({ kind: "mask", tabId: activeTabContext().tabId, select: true })],
  ["cm_ShrinkSelection", () => ui().openDialog({ kind: "mask", tabId: activeTabContext().tabId, select: false })],
];

export function registerFileOpCommands() {
  for (const [id, run] of DEFS) registerCommand({ id, titleKey: `commands.${id}`, when: "fileList", run });
}
