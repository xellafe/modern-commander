// Cache delle righe del viewer per scheda. Una Map di modulo e non uno store reattivo: nessun rendering
// dipende dai suoi cambiamenti, serve solo a non rifare scansioni già fatte.
import { codeUnit } from "../lib/encoding";
import type { Mark } from "../ipc/types";

export interface LineCache {
  encoding: string;
  /** Inizio riga → inizio della successiva. */
  next: Map<number, number>;
  /** Inizio riga → offset fino a cui non c'è LF. */
  fwd: Map<number, number>;
  /** Inizio di una riga → offset più basso esaminato senza LF cercando la riga sopra. */
  back: Map<number, number>;
  /** Inizio riga → punti di controllo (offset, colonna). */
  marks: Map<number, Mark[]>;
  /** Inizio riga → colonne totali. */
  widths: Map<number, number>;
}

const caches = new Map<string, LineCache>();

/** La cache della scheda, creata se manca. Gli offset non dipendono dalla codifica salvo l'unità di codice; le colonne sì. */
export function lineCache(tabId: string, encoding: string): LineCache {
  let c = caches.get(tabId);
  if (!c) {
    c = { encoding, next: new Map(), fwd: new Map(), back: new Map(), marks: new Map(), widths: new Map() };
    caches.set(tabId, c);
    return c;
  }
  if (c.encoding !== encoding) {
    // Gli LF dipendono dall'unità e, in UTF-16, dall'ordine dei byte.
    if (codeUnit(c.encoding) !== codeUnit(encoding) || (c.encoding === "utf-16be") !== (encoding === "utf-16be")) {
      c.next.clear();
      c.fwd.clear();
      c.back.clear();
    }
    c.marks.clear();
    c.widths.clear();
    c.encoding = encoding;
  }
  return c;
}

export function disposeLineCache(tabId: string): void {
  caches.delete(tabId);
}
