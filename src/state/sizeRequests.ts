// Richieste di calcolo della dimensione in corso. Stato non reattivo: l'effetto sulla vista
// (`sizing`, `dirSizes`) lo applica chi chiama; qui solo id, evento di ritorno e annullamento.
import { calcDirSize, cancelDirSize, onDirSize } from "../ipc/api";
import type { AppError, DirSize } from "../ipc/types";

interface Pending {
  tabId: string;
  resolve: (r: DirSize | null) => void;
  reject: (e: AppError) => void;
}

let nextId = 1;
const pending = new Map<number, Pending>();
let listening: Promise<unknown> | undefined;

// Un solo ascoltatore per tutta l'app, registrato alla prima richiesta.
const listen = () =>
  (listening ??= onDirSize(({ requestId, result, error }) => {
    const p = pending.get(requestId);
    if (!p) return; // annullata: la risposta CANCELLED non serve
    pending.delete(requestId);
    if (result) p.resolve(result);
    else p.reject(error ?? { code: "IO", path: null, detail: null });
  }));

/** Dimensione di `path`. Si risolve con null se la richiesta viene annullata (`cancelTabSizes`), si rifiuta con l'AppError della radice. */
export async function computeDirSize(tabId: string, path: string): Promise<DirSize | null> {
  await listen();
  const id = nextId++;
  const done = new Promise<DirSize | null>((resolve, reject) => pending.set(id, { tabId, resolve, reject }));
  try {
    await calcDirSize(id, path);
  } catch (e) {
    pending.delete(id);
    throw e;
  }
  return done;
}

/** Annulla tutti i calcoli della scheda (navigazione o chiusura). */
export function cancelTabSizes(tabId: string) {
  for (const [id, p] of pending) {
    if (p.tabId !== tabId) continue;
    pending.delete(id);
    p.resolve(null);
    void cancelDirSize(id).catch(() => {});
  }
}
