import { create } from "zustand";

const MAX_RECENT = 50;

/** `path` in testa (le più recenti prima), senza doppioni (Windows non distingue le maiuscole), al massimo `max` voci. */
export function pushRecent(list: string[], path: string, max = MAX_RECENT): string[] {
  const key = path.toLowerCase();
  return [path, ...list.filter((p) => p.toLowerCase() !== key)].slice(0, max);
}

interface RecentDirsState {
  dirs: string[];
  push: (path: string) => void;
}

// Solo per la sessione: non si salva su disco.
export const useRecentDirs = create<RecentDirsState>((set) => ({
  dirs: [],
  push: (path) => set((s) => ({ dirs: pushRecent(s.dirs, path) })),
}));
