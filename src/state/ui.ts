import { create } from "zustand";
import type { ConflictPolicy } from "../ipc/types";

export type SidebarView = "explorer" | "search" | "git" | "favorites";

/** Pagine a schermo intero sopra l'area dei gruppi. */
export type PageKind = "settings" | "keybindings";

export const SIDEBAR_MIN = 160;
export const SIDEBAR_MAX = 480;
export const PANEL_MIN = 100;
export const PANEL_MAX_RATIO = 0.6;

/** Gruppo scelto come destinazione con i tasti 1…9 (`n` = posizione nel layout). */
export interface CopyTarget { n: number; groupId: string; path: string }

/** Finestre aperte dai comandi; i prompt della coda stanno in `useJobs`. */
export type DialogRequest =
  | {
      kind: "copyMove";
      move: boolean;
      /** Percorsi completi delle sorgenti. */
      sources: string[];
      /** Cartella delle sorgenti: base dei percorsi relativi. */
      from: string;
      dest: string;
      /** Un solo gruppo: OK resta disattivato finché la destinazione è `from`. */
      requireChange: boolean;
      targets: CopyTarget[];
      policy: ConflictPolicy["type"];
    }
  | { kind: "mkdir"; tabId: string; path: string }
  | { kind: "delete"; permanent: boolean; sources: string[] }
  | { kind: "mask"; tabId: string; select: boolean };

const clamp = (v: number, min: number, max: number) => Math.min(Math.max(v, min), max);

interface UiState {
  sidebarVisible: boolean;
  sidebarView: SidebarView;
  sidebarWidth: number;
  bottomPanelVisible: boolean;
  bottomPanelTab: "operations" | "terminal";
  bottomPanelHeight: number;
  dialog: DialogRequest | null;
  page: PageKind | null;
  openPage: (p: PageKind) => void;
  closePage: () => void;
  /** Una finestra alla volta: se ce n'è già una aperta la richiesta è ignorata. */
  openDialog: (d: DialogRequest) => void;
  closeDialog: () => void;
  toggleSidebar: () => void;
  toggleBottomPanel: () => void;
  /** Cambia vista e mostra la barra laterale se era nascosta. */
  setSidebarView: (v: SidebarView) => void;
  setSidebarWidth: (w: number) => void;
  setBottomPanelHeight: (h: number) => void;
}

// Dimensioni non persistite in questa fase.
export const useUi = create<UiState>((set) => ({
  sidebarVisible: true,
  sidebarView: "explorer",
  sidebarWidth: 220,
  bottomPanelVisible: true,
  bottomPanelTab: "operations",
  bottomPanelHeight: 180,
  dialog: null,
  page: null,
  openPage: (page) => set({ page }),
  closePage: () => set({ page: null }),
  openDialog: (d) => set((s) => (s.dialog ? s : { dialog: d })),
  closeDialog: () => set({ dialog: null }),
  toggleSidebar: () => set((s) => ({ sidebarVisible: !s.sidebarVisible })),
  toggleBottomPanel: () => set((s) => ({ bottomPanelVisible: !s.bottomPanelVisible })),
  setSidebarView: (sidebarView) => set({ sidebarView, sidebarVisible: true }),
  setSidebarWidth: (w) => set({ sidebarWidth: clamp(w, SIDEBAR_MIN, SIDEBAR_MAX) }),
  setBottomPanelHeight: (h) =>
    set({ bottomPanelHeight: clamp(h, PANEL_MIN, Math.max(PANEL_MIN, window.innerHeight * PANEL_MAX_RATIO)) }),
}));
