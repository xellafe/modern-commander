// Comandi sui file (Task 16): copia, spostamento, rinomina, nuova cartella, eliminazione, selezione per maschera.
import { enqueueJob, normalizePaths } from "../ipc/api";
import type { ConflictPolicy, JobId, JobSpec } from "../ipc/types";
import { DRIVES, joinPath, normalizePath } from "../lib/paths";
import { useJobs, waitForJob } from "../state/jobs";
import { targetGroup, useLayout, type LayoutState } from "../state/layout";
import { useSettings } from "../state/settings";
import { parentPath, selectByMask } from "../state/tabView";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { useUi, type CopyTarget, type DialogRequest } from "../state/ui";
import { registerCommand, type Command } from "./registry";

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

/** Chiave i18n dell'errore di un nome di file o cartella, `null` se è valido. */
export function nameError(name: string): string | null {
  if (!name.trim()) return "ops.nameEmpty";
  // Windows toglie in silenzio punti e spazi finali: meglio rifiutarli che creare un nome diverso.
  return INVALID_CHARS.test(name) || /[. ]$/.test(name) ? "ops.nameInvalid" : null;
}

const ABS_ROOT = /^(?:[A-Za-z]:\\|\\\\[^\\/]+\\[^\\/]+\\)/;

/**
 * Percorso da creare con F7: relativo alla cartella corrente (con `\` per le intermedie) o assoluto.
 * `focus` è il nome, dentro la cartella corrente, su cui portare il cursore dopo la creazione.
 */
export function mkdirTarget(base: string, input: string): { path: string; focus: string | null } | { error: string } {
  const s = input.trim();
  const root = ABS_ROOT.exec(s)?.[0] ?? "";
  if (!root && base === DRIVES) return { error: "ops.nameInvalid" };
  const parts = s.slice(root.length).split("\\").filter(Boolean);
  if (!parts.length) return { error: "ops.nameEmpty" };
  for (const p of parts) {
    const e = nameError(p);
    if (e) return { error: e };
  }
  const rel = parts.join("\\");
  if (!root) return { path: joinPath(base, rel), focus: parts[0] };
  const path = root + rel;
  return { path, focus: parentPath(path)?.toLowerCase() === base.toLowerCase() ? parts[parts.length - 1] : null };
}

/** Destinazione digitata nella finestra di copia: assoluta, oppure relativa alla cartella delle sorgenti. */
export function resolveDest(base: string, input: string): string | null {
  if (!input.trim()) return null;
  const p = normalizePath(input);
  if (/^(?:[A-Za-z]:\\|\\\\)/.test(p)) return p;
  return base === DRIVES ? null : joinPath(base, p);
}

/** OK disattivato: destinazione uguale alla cartella delle sorgenti con un solo gruppo o per uno spostamento. */
export const mustChangeDest = (req: { from: string; move: boolean; requireChange: boolean }, dest: string | null) =>
  (req.requireChange || req.move) && dest?.toLowerCase() === req.from.toLowerCase();

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
  if (ctx.path === DRIVES) return null;
  const sources = ctx.markedOrCursor.map((r) => joinPath(ctx.path, r.name));
  return sources.length ? { ...ctx, sources } : null;
}

function openCopyMove(move: boolean, samePanel: boolean) {
  const sel = selection();
  if (!sel) return;
  // Shift+F5: copia nella stessa cartella, con un nome nuovo (o il prompt di conflitto se si sceglie "Chiedi").
  const t = samePanel ? { dest: sel.path, requireChange: false, targets: [] } : copyTargets(useLayout.getState());
  ui().openDialog({
    kind: "copyMove", move, sources: sel.sources, from: sel.path,
    dest: t.dest === DRIVES ? "" : t.dest, requireChange: t.requireChange, targets: t.targets,
    policy: samePanel ? "renameNew" : "ask",
  });
}

export function startCopyMove(
  req: Extract<DialogRequest, { kind: "copyMove" }>,
  dest: string,
  filter: string,
  policy: ConflictPolicy["type"],
) {
  return enqueue({
    kind: { type: req.move ? "move" : "copy" }, sources: req.sources, dest,
    filter: filter.trim() || null, policy: { type: policy },
  });
}

/** Drop da Explorer su un gruppo: finestra di copia verso la cartella della sua scheda attiva. */
export async function dropOnGroup(groupId: string, paths: string[]) {
  const l = useLayout.getState();
  const g = l.groups.find((x) => x.id === groupId);
  // Con una finestra o un prompt aperti il drop si ignora: non si perde ciò che l'utente sta scrivendo.
  if (!g || !paths.length || ui().dialog || useJobs.getState().prompts.length) return;
  const sources = await normalizePaths(paths);
  const dest = l.tabs[g.activeTabId].path;
  ui().openDialog({
    kind: "copyMove", move: false, sources, from: parentPath(sources[0]) ?? sources[0],
    dest: dest === DRIVES ? "" : dest, requireChange: false, targets: [], policy: "ask",
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
  if (newName === oldName || nameError(newName)) return;
  const id = await enqueue({ kind: { type: "rename" }, sources: [joinPath(base, oldName)], dest: joinPath(base, newName) });
  await focusWhenDone(id, tabId, base, newName);
}

export const deleteSources = (sources: string[], permanent: boolean) =>
  enqueue({ kind: { type: "delete", permanent }, sources, dest: null });

async function openDelete(permanent: boolean) {
  const sel = selection();
  if (!sel) return;
  if (useSettings.getState().confirmDelete) ui().openDialog({ kind: "delete", permanent, sources: sel.sources });
  else await deleteSources(sel.sources, permanent);
}

export const applyMask = (tabId: string, mask: string, select: boolean) =>
  views().update(tabId, (v) => selectByMask(v, mask, select));

function startRename() {
  const { tabId, path, cursorRow } = activeTabContext();
  if (path === DRIVES || !cursorRow || cursorRow.isParent) return;
  views().setOverlay({ kind: "rename", tabId, name: cursorRow.name });
}

function openMkdir() {
  const { tabId, path } = activeTabContext();
  if (path !== DRIVES) ui().openDialog({ kind: "mkdir", tabId, path });
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
