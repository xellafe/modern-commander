import { create } from "zustand";
import { listDrives } from "../ipc/api";
import type { Drive } from "../ipc/types";

interface DrivesState {
  drives: Drive[];
  /** Rilegge le unità; se una lettura precedente non è ancora tornata non ne parte un'altra. */
  refresh: () => Promise<void>;
}

// Una condivisione di rete irraggiungibile può bloccare il backend a lungo: senza questa guardia
// ogni giro del timer accoderebbe un'altra chiamata.
let inFlight = false;

export const useDrives = create<DrivesState>((set) => ({
  drives: [],
  refresh: async () => {
    if (inFlight) return;
    inFlight = true;
    try {
      set({ drives: await listDrives() });
    } catch {
      // si tiene l'elenco precedente: al prossimo giro si riprova
    } finally {
      inFlight = false;
    }
  },
}));
