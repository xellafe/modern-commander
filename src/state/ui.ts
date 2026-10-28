import { create } from "zustand";
import type { ConflictPolicy, Connection } from "../ipc/types";
import type { SaveErrorCode, SaveOptions } from "./editors";

export type SidebarView = "explorer" | "search" | "git" | "favorites";

/** Pagine a schermo intero sopra l'area dei gruppi. */
export type PageKind = "settings" | "keybindings";

export const SIDEBAR_MIN = 160;
export const SIDEBAR_MAX = 480;
export const PANEL_MIN = 100;
export const PANEL_MAX_RATIO = 0.6;
export const INSPECTOR_MIN = 220;
export const INSPECTOR_MAX_RATIO = 0.5;

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
  /** Alt+F5: crea uno ZIP con le `sources`; `name` e `destDir` sono la proposta, modificabile. */
  | { kind: "pack"; sources: string[]; name: string; destDir: string }
  | { kind: "delete"; permanent: boolean; sources: string[] }
  | { kind: "mask"; tabId: string; select: boolean }
  /** Chiusura di una scheda editor con modifiche non salvate. */
  | { kind: "unsaved"; tabId: string }
  /**
   * Salvataggio rifiutato (file cambiato su disco, caratteri che la codifica non rappresenta, server senza
   * scrittura sicura) o da confermare (date del server poco affidabili). `opts`: le opzioni del tentativo, a cui
   * il dialogo aggiunge la correzione scelta.
   */
  | { kind: "saveError"; tabId: string; code: SaveErrorCode; opts: SaveOptions }
  /** Chiusura della finestra con `count` operazioni non finite e `dirty` file non salvati. */
  | { kind: "exit"; count: number; dirty: number }
  /** Gestore delle connessioni (Ctrl+F); `draft`: connessione nuova già compilata da un indirizzo scritto a mano. */
  | { kind: "connections"; draft?: Partial<Connection> }
  /**
   * Richieste di `ensureConnected`: `done` chiude la finestra e risponde (null = Annulla). La password resta nello
   * stato locale del dialogo e passa solo a `done`.
   */
  | { kind: "password"; id: string; failed: boolean; done: (r: { password: string; remember: boolean } | null) => void }
  /** Impronta sconosciuta: chiave SSH oppure certificato TLS (`tls`). */
  | { kind: "trust"; id: string; tls: boolean; fingerprint: string; done: (accept: boolean) => void }
  /** Avviso con il solo OK; `title` e `text` sono chiavi i18n, `values` i loro parametri. */
  | { kind: "message"; title: string; text: string; values?: Record<string, string> };

const clamp = (v: number, min: number, max: number) => Math.min(Math.max(v, min), max);

interface UiState {
  sidebarVisible: boolean;
  sidebarView: SidebarView;
  sidebarWidth: number;
  bottomPanelVisible: boolean;
  bottomPanelTab: "operations" | "terminal";
  bottomPanelHeight: number;
  inspectorVisible: boolean;
  inspectorWidth: number;
  dialog: DialogRequest | null;
  page: PageKind | null;
  openPage: (p: PageKind) => void;
  closePage: () => void;
  /** Una finestra alla volta: se ce n'è già una aperta la richiesta è ignorata. */
  openDialog: (d: DialogRequest) => void;
  closeDialog: () => void;
  toggleSidebar: () => void;
  toggleBottomPanel: () => void;
  /** Mostra il pannello in basso sulla scheda `tab`. */
  showBottomPanel: (tab: "operations" | "terminal") => void;
  toggleInspector: () => void;
  /** Cambia vista e mostra la barra laterale se era nascosta. */
  setSidebarView: (v: SidebarView) => void;
  setSidebarWidth: (w: number) => void;
  setBottomPanelHeight: (h: number) => void;
  setInspectorWidth: (w: number) => void;
}

// Dimensioni non persistite in questa fase.
export const useUi = create<UiState>((set) => ({
  sidebarVisible: true,
  sidebarView: "explorer",
  sidebarWidth: 220,
  bottomPanelVisible: true,
  bottomPanelTab: "operations",
  bottomPanelHeight: 180,
  inspectorVisible: false,
  inspectorWidth: 320,
  dialog: null,
  page: null,
  openPage: (page) => set({ page }),
  closePage: () => set({ page: null }),
  openDialog: (d) => set((s) => (s.dialog ? s : { dialog: d })),
  closeDialog: () => set({ dialog: null }),
  toggleSidebar: () => set((s) => ({ sidebarVisible: !s.sidebarVisible })),
  toggleBottomPanel: () => set((s) => ({ bottomPanelVisible: !s.bottomPanelVisible })),
  showBottomPanel: (bottomPanelTab) => set({ bottomPanelVisible: true, bottomPanelTab }),
  toggleInspector: () => set((s) => ({ inspectorVisible: !s.inspectorVisible })),
  setSidebarView: (sidebarView) => set({ sidebarView, sidebarVisible: true }),
  setSidebarWidth: (w) => set({ sidebarWidth: clamp(w, SIDEBAR_MIN, SIDEBAR_MAX) }),
  setBottomPanelHeight: (h) =>
    set({ bottomPanelHeight: clamp(h, PANEL_MIN, Math.max(PANEL_MIN, window.innerHeight * PANEL_MAX_RATIO)) }),
  setInspectorWidth: (w) =>
    set({ inspectorWidth: clamp(w, INSPECTOR_MIN, Math.max(INSPECTOR_MIN, window.innerWidth * INSPECTOR_MAX_RATIO)) }),
}));
