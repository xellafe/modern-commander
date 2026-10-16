import { useEffect } from "react";
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

/** Cartella su un'unità che il backend non osserva (rimovibile o CD: il watcher ne bloccherebbe la rimozione). */
export const isUnwatched = (path: string, drives: Drive[]) =>
  drives.some((d) => (d.kind === "removable" || d.kind === "cdrom") && path.toLowerCase().startsWith(d.root.toLowerCase()));

const DRIVES_REFRESH_MS = 10_000;

/** Rilettura periodica delle unità (ogni 10 s e al ritorno in primo piano). Da montare una sola volta: la barra di stato. */
export function useDrivesPolling() {
  useEffect(() => {
    const refresh = () => void useDrives.getState().refresh();
    refresh();
    const timer = setInterval(refresh, DRIVES_REFRESH_MS);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
    };
  }, []);
}
