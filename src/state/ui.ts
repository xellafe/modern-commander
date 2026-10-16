import { create } from "zustand";

export type SidebarView = "explorer" | "search" | "git" | "favorites" | "settings";

export const SIDEBAR_MIN = 160;
export const SIDEBAR_MAX = 480;
export const PANEL_MIN = 100;
export const PANEL_MAX_RATIO = 0.6;

const clamp = (v: number, min: number, max: number) => Math.min(Math.max(v, min), max);

interface UiState {
  sidebarVisible: boolean;
  sidebarView: SidebarView;
  sidebarWidth: number;
  bottomPanelVisible: boolean;
  bottomPanelTab: "operations" | "terminal";
  bottomPanelHeight: number;
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
  toggleSidebar: () => set((s) => ({ sidebarVisible: !s.sidebarVisible })),
  toggleBottomPanel: () => set((s) => ({ bottomPanelVisible: !s.bottomPanelVisible })),
  setSidebarView: (sidebarView) => set({ sidebarView, sidebarVisible: true }),
  setSidebarWidth: (w) => set({ sidebarWidth: clamp(w, SIDEBAR_MIN, SIDEBAR_MAX) }),
  setBottomPanelHeight: (h) =>
    set({ bottomPanelHeight: clamp(h, PANEL_MIN, Math.max(PANEL_MIN, window.innerHeight * PANEL_MAX_RATIO)) }),
}));
