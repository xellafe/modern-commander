// Integrazione con la shell (Task 18): menu contestuale nativo, Proprietà, drag in uscita, apertura dei file.
import type { MouseEvent } from "react";
import i18n from "../i18n";
import { openPath, openWith, shiftPressed, showContextMenu, showProperties, startDrag } from "../ipc/api";
import type { AppError } from "../ipc/types";
import { isArchive, isVirtual, joinPath, sourceCaps, type SourceCaps } from "../lib/paths";
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

/** Percorsi per la shell; senza righe la cartella corrente stessa. Nella lista delle unità solo le unità, sul remoto e negli archivi niente. */
export function shellPaths(dir: string, rows: Row[]): string[] {
  const shell = sourceCaps(dir).shell;
  if (!rows.length) return shell ? [dir] : [];
  return rows.flatMap((r) => (r.drive ? [r.name] : shell ? [joinPath(dir, r.name)] : []));
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

/** Voci del menu dell'app (schede remote, senza shell): dipendono da cosa si può fare sulla sorgente e da quante righe. */
export function appMenuItems(caps: SourceCaps, rows: Row[]): { commandId: string; labelKey: string }[] {
  const one = rows.length === 1;
  const ids = [
    ["cm_List", true],
    ["cm_Edit", caps.write && one && !rows[0].isDir],
    ["cm_Copy", true],
    ["cm_RenMov", caps.write],
    ["cm_RenameOnly", caps.write && one],
    ["cm_Delete", caps.write],
    ["cm_MkDir", caps.write],
    ["cm_RereadSource", true],
    ["mc_CopyPath", true],
  ] as const;
  return ids.filter(([, on]) => on).map(([commandId]) => ({ commandId, labelKey: `commands.${commandId}` }));
}

/** Dopo un'azione del menu (Elimina, Rinomina…) la lista si aggiorna da sola col watcher. Sul remoto e negli archivi il menu è dell'app. */
function showMenu(tabId: string, rows: Row[], x: number, y: number) {
  const dir = useLayout.getState().tabs[tabId].path;
  if (isVirtual(dir)) return useTabViews.getState().setOverlay({ kind: "appMenu", tabId, items: appMenuItems(sourceCaps(dir), rows), x, y });
  const paths = shellPaths(dir, rows);
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
  if (isVirtual(useLayout.getState().tabs[tabId].path)) {
    // Sul remoto le voci devono riguardare le righe su cui agiranno i comandi (marcate, altrimenti il cursore), come in TC:
    // il tasto destro su una riga non marcata toglie le marcature e la fa unico bersaglio.
    const row = i === null ? undefined : v.rows[i];
    if (row && !v.marked.has(row.name) && v.marked.size) views.update(tabId, (w) => ({ ...w, marked: new Set() }));
    const w = useTabViews.getState().views[tabId];
    const rows = w.marked.size ? w.rows.filter((r) => w.marked.has(r.name)) : (w.rows[w.cursor] && !w.rows[w.cursor].isParent ? [w.rows[w.cursor]] : []);
    return void showMenu(tabId, rows, e.clientX, e.clientY);
  }
  void showMenu(tabId, i === null ? [] : rowsAt(v, i), e.clientX, e.clientY);
}

/**
 * Listener globale di `contextmenu`: il menu di WebView2 (Ricarica, Stampa, Ispeziona…) non compare mai fuori
 * dai campi di testo, dove resta il menu di modifica. Il tasto Menu apre cm_ContextMenu al keydown (mappa tasti) e
 * al keyup Chromium genera un contextmenu senza pulsante (`button` -1). Con `keepMouseMenu` (sviluppo) il tasto
 * destro lo lascia, per Ispeziona.
 */
export function suppressNativeContextMenu(e: Pick<globalThis.MouseEvent, "button" | "target" | "preventDefault">, keepMouseMenu: boolean) {
  const t = e.target as Element | null;
  const editable = typeof t?.closest === "function" && t.closest('input, textarea, [contenteditable]:not([contenteditable="false"])');
  if (!editable && !(keepMouseMenu && e.button === 2)) e.preventDefault();
}

interface SelfDragState {
  dragging: boolean;
  /** Percorsi dell'ultimo drag e istante in cui è finito. */
  last: { paths: string[]; at: number } | null;
  /** Gruppo e cartella da cui è partito l'ultimo drag. */
  origin?: { groupId: string; from: string } | null;
  /** Cosa si trascina («nome» o "N elementi"), per l'immagine sotto il cursore. */
  what?: string;
}
const SELF_DROP_MS = 2000;
const selfDrag: SelfDragState = { dragging: false, last: null, origin: null };

const describe = (rows: Row[]) =>
  rows.length === 1 ? i18n.t("ops.oneItem", { name: rows[0].name }) : i18n.t("ops.items", { count: rows.length });

// Immagine del trascinamento sopra la finestra dell'app: quella della shell si vede solo fuori (sopra Explorer),
// la WebView non la disegna; il drag tra gruppi delle schede remote non ne ha nessuna.
let ghost: HTMLDivElement | null = null;
// Sale a ogni chiusura: una risposta di `shiftPressed` partita prima non riapre l'immagine.
let ghostGen = 0;
// Dopo il drop gli `over` ancora in coda non la riaprono, fino al drag successivo.
let ghostDropped = false;

function showGhost(x: number, y: number, move: boolean) {
  if (!ghost) {
    ghost = document.createElement("div");
    ghost.className = "mc-drag-ghost";
    document.body.appendChild(ghost);
  }
  ghost.textContent = i18n.t(move ? "ops.dragMove" : "ops.dragCopy", { what: selfDrag.what });
  ghost.style.transform = `translate(${x + 14}px, ${y + 12}px)`;
}

export function hideGhost(dropped = false) {
  ghost?.remove();
  ghost = null;
  ghostGen++;
  if (dropped) ghostDropped = true;
}

/** Il nostro drag nativo passa sopra la finestra (`over` di Tauri, in pixel CSS): immagine e Maiusc letto dal sistema. */
export function ghostOver(x: number, y: number) {
  if (!selfDrag.dragging || ghostDropped) return;
  const gen = ghostGen;
  shiftPressed().then((move) => gen === ghostGen && !ghostDropped && selfDrag.dragging && showGhost(x, y, move), console.error);
}

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
 * ignora; su un altro gruppo è una copia dalla cartella d'origine, o uno spostamento con Maiusc (`dropOnGroup`).
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
  if (!paths.length) return;
  selfDrag.origin = { groupId: l.groups.find((g) => g.tabIds.includes(tabId))?.id ?? "", from };
  selfDrag.what = describe(rows);
  ghostDropped = false;
  selfDrag.dragging = true;
  try {
    await startDrag(paths);
  } catch (e) {
    report(tabId, e);
  } finally {
    hideGhost();
    selfDrag.last = { paths, at: Date.now() };
    // Un macrotask dopo: un drop già in coda viene comunque scartato.
    setTimeout(() => (selfDrag.dragging = false), 0);
  }
}

/** `dropOnGroup`; `move` assente = si legge Maiusc dal sistema. */
type DropFn = (groupId: string, paths: string[], move?: boolean) => Promise<void>;

/**
 * Sulle schede remote non c'è il drag OLE: le righe si portano con il mouse su un altro gruppo dell'app, che al
 * rilascio apre la finestra di copia (`drop` è `dropOnGroup`, passata da chi chiama per non incrociare i moduli).
 */
async function dragInApp(tabId: string, name: string, drop: DropFn) {
  const v = useTabViews.getState().views[tabId];
  const rows = v ? rowsAt(v, v.rows.findIndex((r) => r.name === name)) : [];
  if (!rows.length) return;
  const l = useLayout.getState();
  const from = l.tabs[tabId].path;
  selfDrag.origin = { groupId: l.groups.find((g) => g.tabIds.includes(tabId))?.id ?? "", from };
  selfDrag.what = describe(rows);
  ghostDropped = false;
  selfDrag.dragging = true;
  try {
    // Rilascio nella finestra (mouseup), oppure fuori (perdita del focus o mousemove a pulsante già rilasciato): annulla.
    const up = await new Promise<globalThis.MouseEvent | null>((res) => {
      const end = (ev: globalThis.MouseEvent | null) => {
        hideGhost();
        for (const [t, h] of Object.entries(on)) window.removeEventListener(t, h as EventListener);
        res(ev);
      };
      const on = {
        mouseup: (ev: globalThis.MouseEvent) => end(ev),
        blur: () => end(null),
        mousemove: (ev: globalThis.MouseEvent) => (ev.buttons & 1 ? showGhost(ev.clientX, ev.clientY, ev.shiftKey) : end(null)),
      };
      for (const [t, h] of Object.entries(on)) window.addEventListener(t, h as EventListener);
    });
    const groupId = up && document.elementFromPoint(up.clientX, up.clientY)?.closest<HTMLElement>("[data-group-id]")?.dataset.groupId;
    // Maiusc al rilascio sposta, come in TC.
    if (groupId) await drop(groupId, rows.map((r) => joinPath(from, r.name)), up.shiftKey);
  } catch (e) {
    report(tabId, e);
  } finally {
    hideGhost();
    selfDrag.dragging = false;
  }
}

/** Dal mousedown sinistro su una riga: oltre la soglia, con il pulsante ancora premuto, parte il drag nativo. */
export function trackDragOut(tabId: string, i: number, e: MouseEvent, drop: DropFn) {
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
    void (isVirtual(useLayout.getState().tabs[tabId].path) ? dragInApp(tabId, name, drop) : dragOut(tabId, name));
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

// Copia negli appunti la posizione completa delle righe (cursore o marcate), una per riga.
function copyPath() {
  const { tabId, path, markedOrCursor } = activeTabContext();
  const text = (markedOrCursor.length ? markedOrCursor.map((r) => joinPath(path, r.name)) : [path]).join("\n");
  return run(tabId, navigator.clipboard.writeText(text));
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
  ["mc_CopyPath", copyPath],
  // F3: viewer interno in una scheda del gruppo attivo. Invio (cm_Return) invece esegue i programmi, come Explorer.
  ["cm_List", () => {
    const f = cursorFile();
    const l = useLayout.getState();
    if (f) l.openFileTab(l.activeGroupId, { path: f.path, mode: "viewer" });
  }],
  // F4: editor interno in una scheda, oppure (impostazione) il programma esterno come nella fase 1. Sul remoto e negli archivi sempre
  // l'interno: l'esterno richiederebbe download, sorveglianza della copia e ricaricamento.
  ["cm_Edit", () => {
    const f = cursorFile();
    if (!f) return;
    // Un archivio non scrivibile (7z, TAR) si apre in sola lettura; il remoto non scrivibile (HTTP) resta un avviso.
    const readOnly = !sourceCaps(f.path).write;
    if (readOnly && !isArchive(f.path)) return useTabViews.getState().showNotice(f.tabId, { code: "READ_ONLY_SOURCE" });
    if (useSettings.getState().useExternalEditor && !isVirtual(f.path)) return editFile(f.tabId, f.path);
    const l = useLayout.getState();
    l.openFileTab(l.activeGroupId, { path: f.path, mode: "editor", ...(readOnly && { readOnly }) });
  }],
];

export const SHELL_COMMAND_IDS = DEFS.map(([id]) => id);

export function registerShellCommands() {
  for (const [id, run] of DEFS) registerCommand({ id, titleKey: `commands.${id}`, when: "fileList", run });
}
