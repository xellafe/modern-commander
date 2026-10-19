// Integrazione con la shell (Task 18): menu contestuale nativo, Proprietà, drag in uscita, apertura dei file.
import type { MouseEvent } from "react";
import { openPath, openWith, showContextMenu, showProperties, startDrag } from "../ipc/api";
import type { AppError } from "../ipc/types";
import { DRIVES, joinPath } from "../lib/paths";
import { useLayout } from "../state/layout";
import { useSettings } from "../state/settings";
import type { Row, TabView } from "../state/tabView";
import { activeTabContext, useTabViews } from "../state/tabViews";
import { registerCommand, type Command } from "./registry";

/** Righe su cui agisce un'azione partita dalla riga `i`: tutte le marcate se `i` è marcata, altrimenti solo `i`. Mai '..'. */
export function rowsAt(view: TabView, i: number): Row[] {
  const row = view.rows[i];
  if (!row || row.isParent) return [];
  return view.marked.has(row.name) ? view.rows.filter((r) => view.marked.has(r.name)) : [row];
}

/** Percorsi per la shell; senza righe la cartella corrente stessa (nessuna nella lista delle unità). */
export function shellPaths(dir: string, rows: Row[]): string[] {
  if (!rows.length) return dir === DRIVES ? [] : [dir];
  return rows.map((r) => (r.drive ? r.name : joinPath(dir, r.name)));
}

/** Da coordinate client CSS a pixel fisici, come le vuole `show_context_menu`. */
export const toPhysical = (x: number, y: number, dpr: number) => ({ x: Math.round(x * dpr), y: Math.round(y * dpr) });

/** Il drag parte solo oltre 4 px: un click resta un click. */
export const pastDragThreshold = (dx: number, dy: number) => dx * dx + dy * dy > 16;

// Un'azione della shell annullata dall'utente non è un errore da mostrare.
function report(tabId: string, e: unknown) {
  if ((e as Partial<AppError> | null)?.code !== "CANCELLED") useTabViews.getState().showNotice(tabId, e);
}

const run = (tabId: string, p: Promise<void>) => p.catch((e) => report(tabId, e));

/** Dopo un'azione del menu (Elimina, Rinomina…) la lista si aggiorna da sola col watcher. */
function showMenu(tabId: string, rows: Row[], x: number, y: number) {
  const paths = shellPaths(useLayout.getState().tabs[tabId].path, rows);
  if (!paths.length) return;
  const pt = toPhysical(x, y, window.devicePixelRatio || 1);
  return run(tabId, showContextMenu(paths, pt.x, pt.y));
}

/**
 * Tasto destro su una riga (`i`) o sull'area vuota della lista (`null`): il cursore va sulla riga, il menu
 * agisce sulle marcate se la riga è marcata, altrimenti solo su di lei; fuori dalle righe (o su '..') sulla cartella.
 */
export function rowContextMenu(tabId: string, i: number | null, e: MouseEvent) {
  e.preventDefault();
  // Il contextmenu "da tastiera" (tasto Menu) non ha il pulsante destro: lo gestisce cm_ContextMenu.
  if (e.button !== 2) return;
  const views = useTabViews.getState();
  const v = views.views[tabId];
  if (!v) return;
  if (i !== null) views.update(tabId, (w) => ({ ...w, cursor: i }));
  void showMenu(tabId, i === null ? [] : rowsAt(v, i), e.clientX, e.clientY);
}

/**
 * Listener globale di `contextmenu`. Il tasto Menu apre cm_ContextMenu al keydown (mappa tasti); al keyup
 * Chromium genera anche un contextmenu senza pulsante (`button` -1) che aprirebbe il menu di WebView2.
 * Nei campi di testo resta il menu di modifica.
 */
export function suppressKeyboardContextMenu(e: Pick<globalThis.MouseEvent, "button" | "target" | "preventDefault">) {
  const t = e.target as Element | null;
  const editable = typeof t?.closest === "function" && t.closest('input, textarea, [contenteditable]:not([contenteditable="false"])');
  if (e.button !== 2 && !editable) e.preventDefault();
}

interface SelfDragState {
  dragging: boolean;
  /** Percorsi dell'ultimo drag e istante in cui è finito. */
  last: { paths: string[]; at: number } | null;
  /** Gruppo e cartella da cui è partito l'ultimo drag. */
  origin?: { groupId: string; from: string } | null;
}
const SELF_DROP_MS = 2000;
const selfDrag: SelfDragState = { dragging: false, last: null, origin: null };

const samePaths = (a: string[], b: string[]) => {
  const set = new Set(a.map((p) => p.toLowerCase()));
  return a.length === b.length && b.every((p) => set.has(p.toLowerCase()));
};

/**
 * Un drop è il nostro drag rilasciato sull'app se il drag è in corso, oppure se porta esattamente i percorsi
 * dell'ultimo drag finito da meno di 2 s: l'evento drop e la risposta di `start_drag` viaggiano su canali diversi.
 */
export const isSelfDrop = (paths: string[], s: SelfDragState, now: number) =>
  s.dragging || (!!s.last && now - s.last.at < SELF_DROP_MS && samePaths(paths, s.last.paths));

export type DropAction = { kind: "external" } | { kind: "ignore" } | { kind: "fromGroup"; from: string };

/**
 * Drop sul gruppo `groupId`: da fuori (Explorer), oppure il nostro drag. Il nostro sullo stesso gruppo si
 * ignora; su un altro gruppo è una copia dalla cartella d'origine (Maiusc per spostare, come in TC, non si
 * può leggere: l'evento di drop di Tauri non porta i modificatori).
 */
export function classifyDrop(paths: string[], s: SelfDragState, now: number, groupId: string): DropAction {
  if (!isSelfDrop(paths, s, now)) return { kind: "external" };
  return s.origin && s.origin.groupId !== groupId ? { kind: "fromGroup", from: s.origin.from } : { kind: "ignore" };
}

/** Per `dropOnGroup`. */
export const dropAction = (paths: string[], groupId: string) => classifyDrop(paths, selfDrag, Date.now(), groupId);

/** Trascina fuori dalla finestra le righe di `rowsAt` dalla riga `name` (mai unità: la shell creerebbe collegamenti). */
export async function dragOut(tabId: string, name: string) {
  const v = useTabViews.getState().views[tabId];
  const rows = v ? rowsAt(v, v.rows.findIndex((r) => r.name === name)).filter((r) => !r.drive) : [];
  if (!rows.length) return;
  const l = useLayout.getState();
  const from = l.tabs[tabId].path;
  const paths = shellPaths(from, rows);
  selfDrag.origin = { groupId: l.groups.find((g) => g.tabIds.includes(tabId))?.id ?? "", from };
  selfDrag.dragging = true;
  try {
    await startDrag(paths);
  } catch (e) {
    report(tabId, e);
  } finally {
    selfDrag.last = { paths, at: Date.now() };
    // Un macrotask dopo: un drop già in coda viene comunque scartato.
    setTimeout(() => (selfDrag.dragging = false), 0);
  }
}

/** Dal mousedown sinistro su una riga: oltre la soglia, con il pulsante ancora premuto, parte il drag nativo. */
export function trackDragOut(tabId: string, i: number, e: MouseEvent) {
  // Il nome, non l'indice: il watcher può ricaricare la lista prima che si superi la soglia.
  const name = useTabViews.getState().views[tabId]?.rows[i]?.name;
  if (e.button !== 0 || name === undefined) return;
  const { clientX: x0, clientY: y0 } = e;
  const stop = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", stop);
  };
  const move = (ev: globalThis.MouseEvent) => {
    if (!(ev.buttons & 1)) return stop();
    if (!pastDragThreshold(ev.clientX - x0, ev.clientY - y0)) return;
    stop();
    void dragOut(tabId, name);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", stop);
}

/** Apre un file col programma associato (Invio, doppio click). */
export const openFile = (tabId: string, path: string) => run(tabId, openPath(path));

/** File sotto il cursore (non cartelle, unità o '..'). */
function cursorFile() {
  const { tabId, path, cursorRow: r } = activeTabContext();
  return r && !r.isDir ? { tabId, name: r.name, path: joinPath(path, r.name) } : null;
}

const editFile = (tabId: string, path: string) => run(tabId, openWith(useSettings.getState().externalEditor, path));

function keyboardMenu() {
  const { tabId, markedOrCursor } = activeTabContext();
  const list = document.querySelector(".mc-group.active .mc-list-scroll");
  const row = list?.querySelector(".mc-row.cur");
  const r = (row ?? list)?.getBoundingClientRect();
  // Sotto la riga del cursore, allineato a sinistra; senza righe in alto a sinistra nella lista.
  if (r) return showMenu(tabId, markedOrCursor, r.left, row ? r.bottom : r.top);
}

function properties() {
  const { tabId, path, markedOrCursor } = activeTabContext();
  const paths = shellPaths(path, markedOrCursor);
  if (paths.length) return run(tabId, showProperties(paths));
}

const DEFS: [string, Command["run"]][] = [
  ["cm_ContextMenu", keyboardMenu],
  ["cm_Properties", properties],
  // F3: viewer interno in una scheda del gruppo attivo. Invio (cm_Return) invece esegue i programmi, come Explorer.
  ["cm_List", () => {
    const f = cursorFile();
    const l = useLayout.getState();
    if (f) l.openFileTab(l.activeGroupId, { path: f.path, mode: "viewer" });
  }],
  ["cm_Edit", () => {
    const f = cursorFile();
    if (f) return editFile(f.tabId, f.path);
  }],
];

export const SHELL_COMMAND_IDS = DEFS.map(([id]) => id);

export function registerShellCommands() {
  for (const [id, run] of DEFS) registerCommand({ id, titleKey: `commands.${id}`, when: "fileList", run });
}
