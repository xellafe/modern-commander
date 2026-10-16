import { create } from "zustand";
import { listDir, listDrives, nearestExistingAncestor, unwatchDir, watchDir } from "../ipc/api";
import type { AppError, Drive, ErrorCode } from "../ipc/types";
import { quickFilterMask } from "../lib/mask";
import { DRIVES } from "../lib/paths";
import { setTabPath, useLayout } from "./layout";
import { useSettings } from "./settings";
import { buildRows, emptyView, jumpTo, navigateTo, type Row, type TabView } from "./tabView";

export interface Notice {
  tabId: string;
  code: ErrorCode;
  path: string | null;
}

export interface MenuItem {
  label: string;
  detail?: string;
  path: string;
  historyIndex?: number;
  current?: boolean;
}

/** Popup aperti sopra la lista di una scheda (uno alla volta). */
export type Overlay =
  | { kind: "quickSearch"; tabId: string; prefix: string }
  | { kind: "filter"; tabId: string }
  | { kind: "menu"; tabId: string; title: string; items: MenuItem[] }
  /** Modifica in place del nome (Shift+F6) sulla riga `name`. */
  | { kind: "rename"; tabId: string; name: string };

export interface LoadOpts {
  /** Nome su cui mettere il cursore (es. la cartella da cui si risale). */
  focus?: string;
  /** Navigazione nella history (indietro/avanti/menu): non aggiunge voci. */
  historyIndex?: number;
  /** Rilettura della stessa cartella: conserva cursore (per nome), marcature e history. */
  reload?: boolean;
}

interface TabViewsState {
  views: Record<string, TabView>;
  notice: Notice | null;
  overlay: Overlay | null;
  load: (tabId: string, path: string, opts?: LoadOpts) => Promise<void>;
  reload: (tabId: string) => Promise<void>;
  update: (tabId: string, fn: (v: TabView) => TabView) => void;
  /** Cambia ordinamento o filtro e ricostruisce le righe dal listing già in memoria. */
  rebuild: (tabId: string, patch: Partial<Pick<TabView, "sort" | "filter">>) => void;
  /** Da chiamare quando una scheda viene chiusa. */
  dispose: (tabId: string) => void;
  showNotice: (tabId: string, error: unknown) => void;
  setOverlay: (o: Overlay | null) => void;
}

// Stato non reattivo per scheda: listing grezzo (per riordinare senza IPC), ultima richiesta
// (le risposte vecchie si scartano), cartella osservata dal watcher.
const listings = new Map<string, Row[]>();
const seq = new Map<string, number>();
const inFlight = new Set<string>();
const reloadAgain = new Set<string>(); // fs-changed arrivato durante un caricamento: si rilegge dopo
const watched = new Map<string, string>();
let noticeTimer: ReturnType<typeof setTimeout> | undefined;

const driveRow = (d: Drive): Row => ({
  name: d.root, isDir: true, size: 0, modifiedMs: 0, attributes: 0, isSymlink: false, drive: d,
});

interface Listing { path: string; entries: Row[]; error?: AppError }

async function fetchListing(path: string): Promise<Listing> {
  if (path === DRIVES) return { path, entries: (await listDrives()).map(driveRow) };
  try {
    return { path, entries: await listDir(path) };
  } catch (e) {
    const err = e as AppError;
    if (err?.code !== "NOT_FOUND" && err?.code !== "NOT_READY") throw e;
    // Cartella sparita o unità non pronta: si risale all'antenato esistente, altrimenti alle unità.
    const up = await nearestExistingAncestor(path);
    return { ...(await fetchListing(up && up !== path ? up : DRIVES)), error: err };
  }
}

function rowsFor(entries: Row[], path: string, v: TabView): Row[] {
  const { showHidden, showSystem } = useSettings.getState();
  return buildRows(entries, path, v.sort, {
    showHidden, showSystem, filter: quickFilterMask(v.filter), dirSizes: v.dirSizes,
  });
}

const indexOrClamp = (rows: Row[], name: string | undefined, fallback: number) => {
  const i = name === undefined ? -1 : rows.findIndex((r) => r.name === name);
  return i >= 0 ? i : Math.max(0, Math.min(fallback, rows.length - 1));
};

// watch_dir è asincrono nel backend (registra il watcher solo alla fine), unwatch_dir no: le chiamate
// di una scheda vanno in fila, altrimenti una watch lenta può arrivare dopo un'altra watch o dopo l'unwatch.
const watchQueue = new Map<string, Promise<void>>();

function queueWatcher(tabId: string, op: () => Promise<void>) {
  // Se il watcher fallisce la lista funziona lo stesso, senza aggiornamento automatico (Ctrl+R).
  const next = (watchQueue.get(tabId) ?? Promise.resolve()).then(op).catch(() => {});
  watchQueue.set(tabId, next);
  void next.then(() => {
    if (watchQueue.get(tabId) === next) watchQueue.delete(tabId);
  });
}

function syncWatcher(tabId: string, path: string) {
  if (watched.get(tabId) === path) return;
  watched.set(tabId, path);
  queueWatcher(tabId, () => (path === DRIVES ? unwatchDir(tabId) : watchDir(tabId, path)));
}

export const useTabViews = create<TabViewsState>((set, get) => ({
  views: {},
  notice: null,
  overlay: null,

  load: async (tabId, path, opts = {}) => {
    const token = (seq.get(tabId) ?? 0) + 1;
    seq.set(tabId, token);
    inFlight.add(tabId);
    const t0 = performance.now();
    try {
      let res: Listing;
      try {
        res = await fetchListing(path);
      } catch (e) {
        if (seq.get(tabId) === token) get().showNotice(tabId, e);
        return;
      }
      if (seq.get(tabId) !== token) return;
      const t1 = performance.now();

      const prev = get().views[tabId];
      const base = prev ?? emptyView(res.path);
      const samePlace = res.path === path;
      const reload = !!opts.reload && samePlace;
      const v = reload
        ? base
        : opts.historyIndex !== undefined && samePlace
          ? jumpTo(base, opts.historyIndex)
          : navigateTo(base, res.path);
      const rows = rowsFor(res.entries, res.path, v);
      const cursor = indexOrClamp(rows, reload ? base.rows[base.cursor]?.name : opts.focus, v.cursor);
      let marked = v.marked;
      if (reload && marked.size) {
        const names = new Set(res.entries.map((e) => e.name));
        marked = new Set([...marked].filter((n) => names.has(n)));
      }

      listings.set(tabId, res.entries);
      set((s) => ({ views: { ...s.views, [tabId]: { ...v, rows, cursor, marked } } }));
      useLayout.setState((s) => setTabPath(s, tabId, res.path));
      if (res.error) get().showNotice(tabId, res.error);
      syncWatcher(tabId, res.path);

      if (import.meta.env.DEV && typeof requestAnimationFrame === "function") {
        const t2 = performance.now();
        // rAF + setTimeout: il callback parte dopo il paint del frame con le righe.
        requestAnimationFrame(() => setTimeout(() => console.log(
          `[perf] list ${res.path}: ${rows.length} righe, ipc ${(t1 - t0).toFixed(1)} ms, ` +
          `buildRows ${(t2 - t1).toFixed(1)} ms, primo frame ${(performance.now() - t0).toFixed(1)} ms`,
        )));
      }
    } finally {
      if (seq.get(tabId) === token) {
        inFlight.delete(tabId);
        if (reloadAgain.delete(tabId)) void get().reload(tabId);
      }
    }
  },

  reload: async (tabId) => {
    const v = get().views[tabId];
    if (!v) return;
    // Non si interrompe un caricamento in corso (potrebbe essere una navigazione): si rilegge alla fine.
    if (inFlight.has(tabId)) {
      reloadAgain.add(tabId);
      return;
    }
    await get().load(tabId, v.history[v.historyIndex], { reload: true });
  },

  update: (tabId, fn) => {
    const v = get().views[tabId];
    if (v) set((s) => ({ views: { ...s.views, [tabId]: fn(v) } }));
  },

  rebuild: (tabId, patch) => {
    const v0 = get().views[tabId];
    if (!v0) return;
    const v = { ...v0, ...patch };
    const rows = rowsFor(listings.get(tabId) ?? [], v.history[v.historyIndex], v);
    const cursor = indexOrClamp(rows, v0.rows[v0.cursor]?.name, v0.cursor);
    set((s) => ({ views: { ...s.views, [tabId]: { ...v, rows, cursor } } }));
  },

  dispose: (tabId) => {
    seq.set(tabId, (seq.get(tabId) ?? 0) + 1); // scarta le risposte in arrivo
    inFlight.delete(tabId);
    reloadAgain.delete(tabId);
    listings.delete(tabId);
    if (watched.delete(tabId)) queueWatcher(tabId, () => unwatchDir(tabId));
    set((s) => {
      const { [tabId]: _gone, ...views } = s.views;
      return { views, overlay: s.overlay?.tabId === tabId ? null : s.overlay };
    });
  },

  showNotice: (tabId, error) => {
    const e = error as Partial<AppError> | null;
    set({ notice: { tabId, code: e?.code ?? "IO", path: e?.path ?? null } });
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => set({ notice: null }), 5000);
  },

  setOverlay: (overlay) => set({ overlay }),
}));

/** Scheda attiva del gruppo attivo: il contesto di tutti i comandi sulla lista. */
export function activeTabContext() {
  const l = useLayout.getState();
  const tabId = l.groups.find((g) => g.id === l.activeGroupId)!.activeTabId;
  const path = l.tabs[tabId].path;
  const view = useTabViews.getState().views[tabId] ?? emptyView(path);
  const cursorRow: Row | undefined = view.rows[view.cursor];
  return {
    tabId, path, view, cursorRow,
    // Calcolato solo se serve: scorre tutte le righe (100.000 in una cartella grande).
    get markedOrCursor(): Row[] {
      const marked = view.marked.size ? view.rows.filter((r) => view.marked.has(r.name)) : [];
      return marked.length ? marked : cursorRow && !cursorRow.isParent ? [cursorRow] : [];
    },
  };
}
