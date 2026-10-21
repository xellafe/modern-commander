// Stato dei viewer F3 per scheda. Sta nello store e non nel componente: cambiando scheda il corpo del
// gruppo si smonta, e al ritorno il viewer riappare allo stesso punto.
import { create } from "zustand";
import { codeUnit, detectEncoding } from "../lib/encoding";
import { fileKind } from "../lib/format";
import { looksBinary, previewKind } from "../lib/preview";
import { disposeLineCache } from "./lineCache";

export type ViewerMode = "text" | "hex" | "image" | "pdf" | "video" | "audio";

export interface ViewerState {
  mode: ViewerMode;
  /** Etichetta `TextDecoder`. */
  encoding: string;
  wrap: boolean;
  /** Offset del primo byte visibile. */
  top: number;
  /** Colonna del primo carattere visibile, con l'a capo spento; uguale per tutte le righe. */
  left: number;
}

/**
 * Vista nativa del file (immagine, PDF, video, audio), se la WebView la mostra. Le immagini che non decodifica
 * (TIFF, HEIC) vanno comunque in `ImagePane`, che ripiega sull'anteprima della shell.
 */
export function nativeMode(name: string): ViewerMode | null {
  const kind = previewKind(name);
  if (kind === "image" || kind === "pdf" || kind === "video" || kind === "audio") return kind;
  return fileKind(name, false) === "image" ? "image" : null;
}

/** Modalità iniziale: la vista nativa se c'è; il resto testo o esadecimale (il Markdown renderizzato è dell'ispettore). */
export function initialViewerState(name: string, sample: Uint8Array, ansi: string, complete: boolean): ViewerState {
  const { encoding, bom } = detectEncoding(sample, ansi, complete);
  const native = nativeMode(name);
  if (native) return { mode: native, encoding, wrap: false, top: bom, left: 0 };
  return looksBinary(sample) ? { mode: "hex", encoding, wrap: false, top: 0, left: 0 } : { mode: "text", encoding, wrap: false, top: bom, left: 0 };
}

/** `top` riportato su un confine valido per la modalità: 16 byte nell'esadecimale, l'unità di codice nel testo. */
export function alignTop(top: number, mode: ViewerMode, encoding: string): number {
  const unit = mode === "hex" ? 16 : codeUnit(encoding);
  return top - (top % unit);
}

interface ViewersStore {
  states: Record<string, ViewerState>;
  set: (tabId: string, patch: Partial<ViewerState>) => void;
  dispose: (tabId: string) => void;
}

export const useViewers = create<ViewersStore>((set) => ({
  states: {},
  set: (tabId, patch) =>
    set((s) => {
      const cur = s.states[tabId];
      // Una modifica tardiva per una scheda chiusa (lettura asincrona finita dopo dispose) non la fa rinascere.
      if (!cur && patch.mode === undefined) return s;
      return { states: { ...s.states, [tabId]: { ...cur, ...patch } as ViewerState } };
    }),
  dispose: (tabId) => {
    disposeLineCache(tabId);
    set((s) => {
      const { [tabId]: _removed, ...states } = s.states;
      return { states };
    });
  },
}));
