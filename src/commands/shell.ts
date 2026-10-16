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
export function suppressKeyboardContextMenu(e: globalThis.MouseEvent) {
  const editable = e.target instanceof Element && e.target.closest('input, textarea, [contenteditable]:not([contenteditable="false"])');
  if (e.button !== 2 && !editable) e.preventDefault();
}

let selfDrag = false;
/** true mentre è in corso un nostro `start_drag`: il drop su noi stessi va ignorato. */
export const isSelfDragging = () => selfDrag;

/** Trascina fuori dalla finestra le righe di `rowsAt(i)`. */
export async function dragOut(tabId: string, i: number) {
  const v = useTabViews.getState().views[tabId];
  const rows = v ? rowsAt(v, i) : [];
  if (!rows.length) return;
  selfDrag = true;
  try {
    await startDrag(shellPaths(useLayout.getState().tabs[tabId].path, rows));
  } catch (e) {
    report(tabId, e);
  } finally {
    selfDrag = false;
  }
}

/** Dal mousedown sinistro su una riga: oltre la soglia, con il pulsante ancora premuto, parte il drag nativo. */
export function trackDragOut(tabId: string, i: number, e: MouseEvent) {
  if (e.button !== 0) return;
  const { clientX: x0, clientY: y0 } = e;
  const stop = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", stop);
  };
  const move = (ev: globalThis.MouseEvent) => {
    if (!(ev.buttons & 1)) return stop();
    if (!pastDragThreshold(ev.clientX - x0, ev.clientY - y0)) return;
    stop();
    void dragOut(tabId, i);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", stop);
}

/** Apre un file col programma associato (Invio, doppio click, F3 in fase 1). */
export const openFile = (tabId: string, path: string) => run(tabId, openPath(path));

/** File sotto il cursore (non cartelle, unità o '..'). */
function cursorFile() {
  const { tabId, path, cursorRow: r } = activeTabContext();
  return r && !r.isDir ? { tabId, path: joinPath(path, r.name) } : null;
}

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
  // Fase 1: programma di default ed editor esterno; in fase 2 viewer ed editor interni.
  ["cm_List", () => {
    const f = cursorFile();
    if (f) return openFile(f.tabId, f.path);
  }],
  ["cm_Edit", () => {
    const f = cursorFile();
    if (f) return run(f.tabId, openWith(useSettings.getState().externalEditor, f.path));
  }],
];

export const SHELL_COMMAND_IDS = DEFS.map(([id]) => id);

export function registerShellCommands() {
  for (const [id, run] of DEFS) registerCommand({ id, titleKey: `commands.${id}`, when: "fileList", run });
}
