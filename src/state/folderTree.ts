import { create } from "zustand";
import { listDir } from "../ipc/api";
import type { Entry, ErrorCode } from "../ipc/types";
import { buildRows } from "./tabView";

export interface TreeNode {
  status: "loading" | "ready" | "error";
  /** Le sole sottocartelle, non filtrate: hidden/system si applicano al disegno, così seguono le impostazioni. */
  dirs: Entry[];
  error?: ErrorCode;
}

interface FolderTreeState {
  /** Percorsi espansi (in memoria: si perdono alla chiusura dell'app). */
  expanded: Record<string, true>;
  nodes: Record<string, TreeNode>;
  toggle: (path: string) => Promise<void>;
  /** Dimentica l'errore di un nodo, così può essere riprovato. */
  retry: (path: string) => void;
}

/** Nomi delle sottocartelle da mostrare: stesso filtro e stesso ordine della lista file. */
export function childDirs(dirs: Entry[], showHidden: boolean, showSystem: boolean): string[] {
  // "C:\" non ha una cartella superiore: buildRows non aggiunge la riga "..".
  return buildRows(dirs, "C:\\", { key: "name", desc: false }, { showHidden, showSystem, filter: "" }).map((r) => r.name);
}

export const useFolderTree = create<FolderTreeState>((set, get) => ({
  expanded: {},
  nodes: {},

  toggle: async (path) => {
    if (get().expanded[path]) {
      set((s) => {
        const { [path]: _gone, ...expanded } = s.expanded;
        return { expanded };
      });
      return;
    }
    // Si mostra subito l'elenco già in memoria (se c'è) mentre si rilegge la cartella.
    set((s) => ({
      expanded: { ...s.expanded, [path]: true },
      nodes: { ...s.nodes, [path]: { status: "loading", dirs: s.nodes[path]?.dirs ?? [] } },
    }));
    try {
      const dirs = (await listDir(path)).filter((e) => e.isDir);
      set((s) => ({ nodes: { ...s.nodes, [path]: { status: "ready", dirs } } }));
    } catch (e) {
      // Il nodo resta chiuso e non espandibile; il tooltip viene da errors.<CODE>.
      set((s) => {
        const { [path]: _gone, ...expanded } = s.expanded;
        return { expanded, nodes: { ...s.nodes, [path]: { status: "error", dirs: [], error: (e as { code?: ErrorCode })?.code ?? "IO" } } };
      });
    }
  },

  retry: (path) =>
    set((s) => {
      if (s.nodes[path]?.status !== "error") return s;
      const { [path]: _gone, ...nodes } = s.nodes;
      return { nodes };
    }),
}));
