// Comandi di navigazione, selezione, schede, ordinamento e vista (Task 11).
import { getCurrentWindow } from "@tauri-apps/api/window";
import i18n from "../i18n";
import { listDrives } from "../ipc/api";
import type { DirSize } from "../ipc/types";
import { formatSize } from "../lib/format";
import { dirTarget, folderName, joinPath, rootPath } from "../lib/paths";
import { cancelAllAndWait, unfinishedJobs, useJobs } from "../state/jobs";
import { targetGroup, useLayout } from "../state/layout";
import {
  finishSizing, hasSize, invertMarks, markRange, parentPath, sizable, startSizing, toggleMark,
  type SortKey, type TabView,
} from "../state/tabView";
import { computeDirSize } from "../state/sizeRequests";
import { activeTabContext, useTabViews, type LoadOpts } from "../state/tabViews";
import { useUi } from "../state/ui";
import { registerCommand, type Command } from "./registry";
import { openFile } from "./shell";

/** Righe visibili nella lista (pagina per PgSu/PgGiù, righe per colonna nella vista breve). */
let pageSize = 20;
export const setPageSize = (n: number) => {
  pageSize = Math.max(1, n);
};

/** Stesso tasto = inverte il verso, come in TC; un tasto nuovo parte crescente. */
export const nextSort = (sort: TabView["sort"], key: SortKey): TabView["sort"] =>
  sort.key === key ? { key, desc: !sort.desc } : { key, desc: false };

const views = () => useTabViews.getState();
const layout = () => useLayout.getState();
const clamp = (i: number, n: number) => Math.max(0, Math.min(i, n - 1));

const open = (path: string, opts?: LoadOpts) => views().load(activeTabContext().tabId, path, opts);
const updateActive = (fn: (v: TabView) => TabView) => views().update(activeTabContext().tabId, fn);
const moveCursor = (to: (v: TabView) => number) => updateActive((v) => ({ ...v, cursor: clamp(to(v), v.rows.length) }));

function markAndMove(delta: number) {
  updateActive((v) => {
    const row = v.rows[v.cursor];
    const marked = row ? toggleMark(v, row.name) : v;
    return { ...marked, cursor: clamp(v.cursor + delta, v.rows.length) };
  });
}

/** Calcoli di dimensione contemporanei per richiesta di gruppo (Alt+Maiusc+Invio): una cartella enorme non apre centinaia di thread. */
const SIZE_CONCURRENCY = 4;

/** Calcola la dimensione delle cartelle `names` di `dir` nella scheda; si ferma se la scheda cambia cartella. */
async function measure(tabId: string, dir: string, names: string[]) {
  const here = () => {
    const v = views().views[tabId];
    return !!v && v.history[v.historyIndex] === dir;
  };
  views().update(tabId, (v) => startSizing(v, names));
  const queue = [...names];
  const worker = async () => {
    for (let name = queue.shift(); name !== undefined; name = queue.shift()) {
      let size: DirSize | null = null;
      try {
        size = await computeDirSize(tabId, joinPath(dir, name));
      } catch (e) {
        if (here()) views().showNotice(tabId, e);
      }
      if (!here()) return; // annullato dalla navigazione: la vista nuova parte da zero
      const done = name;
      views().update(tabId, (v) => finishSizing(v, done, size));
      // L'ordinamento per dimensione dipende dai valori appena arrivati.
      if (views().views[tabId].sort.key === "size") views().rebuild(tabId, {});
    }
  };
  await Promise.all(Array.from({ length: Math.min(SIZE_CONCURRENCY, names.length) }, worker));
}

/** Spazio: marca la riga (il cursore resta fermo); se è una cartella che diventa marcata ne calcola la dimensione. */
function markOrSize() {
  const { tabId, path, view, cursorRow: row } = activeTabContext();
  if (!row || row.isParent || row.drive) return;
  updateActive((v) => toggleMark(v, row.name));
  // Se si smarca la cartella, la dimensione già mostrata resta, come in TC.
  if (sizable(row) && !view.marked.has(row.name) && !view.sizing.has(row.name)) void measure(tabId, path, [row.name]);
}

function countDirContent() {
  const { tabId, path, view } = activeTabContext();
  const names = view.rows
    .filter((r) => sizable(r) && !hasSize(view.dirSizes, r.name) && !view.sizing.has(r.name))
    .map((r) => r.name);
  if (names.length) void measure(tabId, path, names);
}

export function sortBy(tabId: string, key: SortKey) {
  const v = views().views[tabId];
  if (v) views().rebuild(tabId, { sort: nextSort(v.sort, key) });
}

/** Chiude la scheda e ne libera lo stato (watcher, listing). L'ultima scheda dell'ultimo gruppo resta. */
export function closeTab(tabId: string) {
  layout().closeTab(tabId);
  if (!layout().tabs[tabId]) views().dispose(tabId);
}

function goHistory(delta: 1 | -1) {
  const { view } = activeTabContext();
  const i = view.historyIndex + delta;
  if (i >= 0 && i < view.history.length) return open(view.history[i], { historyIndex: i });
}

function openInAdjacentGroup(delta: 1 | -1) {
  const { path, cursorRow } = activeTabContext();
  const target = (cursorRow && dirTarget(path, cursorRow)) ?? path;
  const l = layout();
  const i = l.groups.findIndex((g) => g.id === l.activeGroupId);
  // Senza un gruppo da quella parte si apre nel gruppo stesso (come Ctrl+←/→ di TC nel pannello omonimo).
  const g = l.groups[i + delta] ?? l.groups[i];
  return views().load(g.activeTabId, target);
}

async function openDrivesMenu() {
  const { tabId, path } = activeTabContext();
  const drives = await listDrives();
  const lng = i18n.language;
  views().setOverlay({
    kind: "menu",
    tabId,
    title: i18n.t("groups.drives"),
    items: drives.map((d) => ({
      label: d.root,
      detail: [d.label, d.ready && i18n.t("groups.free", { size: formatSize(d.free, lng) })].filter(Boolean).join(" · "),
      path: d.root,
      current: rootPath(path) === d.root,
    })),
  });
}

function openHistoryMenu() {
  const { tabId, view } = activeTabContext();
  const items = view.history.map((p, i) => ({ label: p, path: p, historyIndex: i, current: i === view.historyIndex }));
  views().setOverlay({ kind: "menu", tabId, title: i18n.t("groups.history"), items: items.reverse() });
}

/** Attesa massima dei job annullati all'uscita: le copie in corso fanno in tempo a togliere il `.mcpart`. */
const EXIT_WAIT_MS = 3000;
let exiting = false;

/**
 * Chiusura della finestra: la X, Alt+F4 e cm_Exit passano tutti di qui. Con operazioni non finite
 * (in coda, in corso, in pausa, in attesa di risposta) si chiede conferma invece di ucciderle.
 */
export const initCloseGuard = () =>
  getCurrentWindow().onCloseRequested((e) => {
    const count = unfinishedJobs(useJobs.getState().jobs).length;
    if (!count && !exiting) return;
    e.preventDefault();
    // Sostituisce un'eventuale altra finestra aperta: l'utente vuole uscire.
    if (!exiting) useUi.setState({ dialog: { kind: "exit", count } });
  });

/** Dalla conferma: annulla tutti i job, ne attende la fine (al massimo `EXIT_WAIT_MS`) e chiude. */
export async function exitCancellingJobs() {
  exiting = true;
  try {
    await cancelAllAndWait(EXIT_WAIT_MS);
    await getCurrentWindow().destroy();
  } finally {
    exiting = false; // se destroy fallisce la finestra resta chiudibile
  }
}

type Def = [id: string, run: Command["run"], opts?: { repeat?: boolean; global?: boolean }];
const R = { repeat: true };

const DEFS: Def[] = [
  // Cursore e marcatura
  ["mc_CursorUp", () => moveCursor((v) => v.cursor - 1), R],
  ["mc_CursorDown", () => moveCursor((v) => v.cursor + 1), R],
  ["mc_CursorPageUp", () => moveCursor((v) => v.cursor - (pageSize - 1)), R],
  ["mc_CursorPageDown", () => moveCursor((v) => v.cursor + (pageSize - 1)), R],
  ["mc_CursorHome", () => moveCursor(() => 0)],
  ["mc_CursorEnd", () => moveCursor((v) => v.rows.length - 1)],
  // ←/→ cambiano colonna solo nella vista breve; nella dettagliata non fanno niente, come in TC.
  ["mc_CursorLeft", () => moveCursor((v) => (v.view === "brief" ? v.cursor - pageSize : v.cursor)), R],
  ["mc_CursorRight", () => moveCursor((v) => (v.view === "brief" ? v.cursor + pageSize : v.cursor)), R],
  ["mc_MarkUp", () => markAndMove(-1), R],
  ["mc_MarkDown", () => markAndMove(1), R],
  ["cm_Reverse", () => markAndMove(1), R],
  ["cm_SelectAll", () => updateActive((v) => markRange(v, 0, v.rows.length - 1))],
  ["cm_ClearAll", () => updateActive((v) => ({ ...v, marked: new Set() }))],
  ["cm_ExchangeSelection", () => updateActive(invertMarks)],
  ["mc_MarkOrSize", markOrSize],
  ["cm_CountDirContent", countDirContent],

  // Navigazione
  ["cm_Return", () => {
    const { tabId, path, cursorRow } = activeTabContext();
    if (!cursorRow) return;
    const target = dirTarget(path, cursorRow);
    if (target) return open(target, cursorRow.isParent ? { focus: folderName(path) } : undefined);
    // Un file (anche un collegamento .lnk) si apre col programma associato, come in Explorer.
    return openFile(tabId, joinPath(path, cursorRow.name));
  }],
  ["cm_GoToParent", () => {
    const { path } = activeTabContext();
    const up = parentPath(path);
    if (up) return open(up, { focus: folderName(path) });
  }, R],
  ["cm_GoToRoot", () => {
    const { path } = activeTabContext();
    if (rootPath(path) !== path) return open(rootPath(path));
  }],
  ["cm_GotoPreviousDir", () => goHistory(-1), R],
  ["cm_GotoNextDir", () => goHistory(1), R],
  ["cm_DirectoryHistory", openHistoryMenu],
  ["cm_RereadSource", () => views().reload(activeTabContext().tabId)],
  ["cm_SrcOpenDrives", openDrivesMenu],
  ["cm_QuickFilter", () => views().setOverlay({ kind: "filter", tabId: activeTabContext().tabId })],

  // Schede e gruppi
  ["cm_OpenNewTab", () => layout().newTab(layout().activeGroupId, activeTabContext().path)],
  ["cm_CloseCurrentTab", () => closeTab(activeTabContext().tabId)],
  ["cm_SwitchToNextTab", () => layout().cycleTab(layout().activeGroupId, 1), R],
  ["cm_SwitchToPreviousTab", () => layout().cycleTab(layout().activeGroupId, -1), R],
  ["mc_FocusNextGroup", () => layout().focusAdjacentGroup(1)],
  ["mc_FocusPrevGroup", () => layout().focusAdjacentGroup(-1)],
  ["mc_SplitGroup", () => layout().splitGroup(layout().activeGroupId)],
  ["mc_OpenInPrevGroup", () => openInAdjacentGroup(-1)],
  ["mc_OpenInNextGroup", () => openInAdjacentGroup(1)],
  ["cm_Exchange", async () => {
    const l = layout();
    const other = targetGroup(l);
    if (!other) return;
    const a = activeTabContext().tabId;
    const b = l.groups.find((g) => g.id === other)!.activeTabId;
    await Promise.all([views().load(a, l.tabs[b].path), views().load(b, l.tabs[a].path)]);
  }],

  // Ordinamento e vista
  ["cm_SrcByName", () => sortBy(activeTabContext().tabId, "name")],
  ["cm_SrcByExt", () => sortBy(activeTabContext().tabId, "ext")],
  ["cm_SrcByDateTime", () => sortBy(activeTabContext().tabId, "date")],
  ["cm_SrcBySize", () => sortBy(activeTabContext().tabId, "size")],
  ["cm_SrcUnsorted", () => sortBy(activeTabContext().tabId, "none")],
  ["cm_SrcShort", () => updateActive((v) => ({ ...v, view: "brief" }))],
  ["cm_SrcLong", () => updateActive((v) => ({ ...v, view: "full" }))],

  // Finestra
  ["mc_ToggleSidebar", () => useUi.getState().toggleSidebar(), { global: true }],
  ["mc_ToggleBottomPanel", () => useUi.getState().toggleBottomPanel(), { global: true }],
  ["cm_Exit", () => getCurrentWindow().close(), { global: true }],
];

export const NAVIGATION_COMMAND_IDS = DEFS.map(([id]) => id);

export function registerNavigationCommands() {
  for (const [id, run, o] of DEFS) {
    registerCommand({ id, titleKey: `commands.${id}`, when: o?.global ? "global" : "fileList", repeat: o?.repeat, run });
  }
}
