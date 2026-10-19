import { create } from "zustand";

/** 'commands' = palette dei comandi (Ctrl+Shift+P); 'jump' = salto rapido a cartelle e file (Ctrl+P). */
export type PaletteMode = "commands" | "jump";

interface PaletteState {
  /** null = chiusa. */
  mode: PaletteMode | null;
  open: (mode: PaletteMode) => void;
  close: () => void;
}

export const usePalette = create<PaletteState>((set) => ({
  mode: null,
  open: (mode) => set({ mode }),
  close: () => set({ mode: null }),
}));
