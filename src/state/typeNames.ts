import { create } from "zustand";
import { typeNames } from "../ipc/api";
import { hasSize } from "./tabView";

/** Descrizioni di Windows dei tipi per chiave di `typeKey`, per tutta la sessione; `""` = chiesto ma senza risposta. */
export const useTypeNames = create<{ names: Record<string, string> }>(() => ({ names: {} }));

// Richieste in volo per chiave: chi chiede una chiave già in volo ne attende l'arrivo (un'altra scheda la sta chiedendo).
const pending = new Map<string, Promise<void>>();

/** Chiede in una sola chiamata le chiavi non ancora note né in volo; true se all'uscita ci sono testi nuovi per `keys`. */
export async function ensureTypeNames(keys: Iterable<string>): Promise<boolean> {
  const known = useTypeNames.getState().names;
  const wanted = [...new Set(keys)].filter((k) => !hasSize(known, k));
  const waiting = [...new Set(wanted.map((k) => pending.get(k)).filter((p) => p !== undefined))];
  const missing = wanted.filter((k) => !pending.has(k));
  if (!missing.length && !waiting.length) return false;
  if (missing.length) {
    const request = (async () => {
      let texts: string[];
      try {
        texts = await typeNames(missing);
      } catch {
        texts = []; // senza risposta il Tipo resta vuoto e non si richiede più
      }
      missing.forEach((k) => pending.delete(k));
      useTypeNames.setState((s) => ({
        names: { ...s.names, ...Object.fromEntries(missing.map((k, i) => [k, texts[i] ?? ""])) },
      }));
    })();
    missing.forEach((k) => pending.set(k, request));
    waiting.push(request);
  }
  await Promise.all(waiting);
  return true;
}
