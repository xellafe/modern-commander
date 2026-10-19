// Righe intere nel viewer: a differenza di textPaging, qui una riga non si spezza a MAX_LINE. Le funzioni
// lavorano sulla finestra di byte già letta; ciò che non sta nella finestra resta ignoto (null) e lo
// risolve il backend (ricerca degli inizi riga e conteggio delle colonne a blocchi).
import type { Mark } from "../ipc/types";
import { isLf, type Win } from "./textPaging";

export interface Line {
  start: number;
  /** Fine del testo (CR o LF esclusi), null se oltre la finestra. */
  end: number | null;
  /** Inizio della riga successiva, null se ignoto. */
  next: number | null;
}

// CR all'indice `i`: `0D` (unit 1), `0D 00` (UTF-16LE) o `00 0D` (UTF-16BE).
function isCr(buf: Uint8Array, i: number, unit: 1 | 2, bigEndian: boolean): boolean {
  if (unit === 1) return buf[i] === 0x0d;
  return bigEndian ? buf[i] === 0 && buf[i + 1] === 0x0d : buf[i] === 0x0d && buf[i + 1] === 0;
}

/** Fino a `max` righe da `top`. Si ferma alla prima riga il cui LF non è nella finestra. */
export function wholeLines(win: Win, top: number, max: number, unit: 1 | 2, bigEndian: boolean): Line[] {
  const { base, buf } = win;
  let pos = top - base;
  if (pos < 0 || pos > buf.length) return [];
  const lines: Line[] = [];
  while (lines.length < max) {
    let lf = -1;
    for (let i = pos; i + unit <= buf.length; i += unit) {
      if (isLf(buf, i, unit, bigEndian)) { lf = i; break; }
    }
    if (lf < 0) {
      // Senza LF la riga arriva a fine file solo se la finestra ci arriva.
      const end = win.eof ? base + buf.length : null;
      lines.push({ start: base + pos, end, next: end });
      break;
    }
    const cr = lf - unit >= pos && isCr(buf, lf - unit, unit, bigEndian);
    lines.push({ start: base + pos, end: base + (cr ? lf - unit : lf), next: base + lf + unit });
    pos = lf + unit;
  }
  return lines;
}

/** Inizio della riga sopra `top`; null se la finestra non basta a determinarlo. */
export function prevLineStart(win: Win, top: number, unit: 1 | 2, bigEndian: boolean): number | null {
  const rel = top - win.base;
  if (rel < 0 || rel > win.buf.length) return null;
  // L'ultima unità prima di `top` è l'LF della riga sopra: si cerca quello prima.
  for (let i = rel - 2 * unit; i >= 0; i -= unit) {
    if (isLf(win.buf, i, unit, bigEndian)) return win.base + i + unit;
  }
  return win.base === 0 ? 0 : null;
}

/** Vero se `at` è 0 o segue un LF; null se il byte prima è fuori dalla finestra. */
export function isLineStart(win: Win, at: number, unit: 1 | 2, bigEndian: boolean): boolean | null {
  if (at === 0) return true;
  const rel = at - win.base - unit;
  if (rel < 0 || rel + unit > win.buf.length) return null;
  return isLf(win.buf, rel, unit, bigEndian);
}

/** Il punto di controllo con `column ≤ target` più avanzato, o l'inizio riga. */
export function nearestMark(marks: Mark[] | undefined, lineStart: number, target: number): Mark {
  let best: Mark = { offset: lineStart, column: 0 };
  for (const m of marks ?? []) if (m.column <= target && m.column > best.column) best = m;
  return best;
}

/** Unione ordinata per `offset`, senza duplicati. */
export function mergeMarks(marks: Mark[] | undefined, add: Mark[]): Mark[] {
  const byOffset = new Map<number, Mark>();
  for (const m of [...(marks ?? []), ...add]) byOffset.set(m.offset, m);
  return [...byOffset.values()].sort((a, b) => a.offset - b.offset);
}

/**
 * `text` parte dalla colonna assoluta `column ≤ left`. Espande i tab sulle colonne assolute (multipli di 8),
 * scarta le colonne `< left` (di un tab tagliato restano gli spazi dopo `left`) e tiene al massimo `width` colonne.
 */
export function renderSlice(text: string, column: number, left: number, width: number): string {
  const end = left + width;
  let col = column;
  let out = "";
  for (const ch of text) {
    if (col >= end) break;
    if (ch === "\t") {
      const stop = Math.min(col + 8 - (col % 8), end);
      out += " ".repeat(Math.max(0, stop - Math.max(col, left)));
      col += 8 - (col % 8);
    } else {
      if (col >= left) out += ch;
      col++;
    }
  }
  return out;
}

export type Nav = { dir: 1 | -1; n: number };

/** Accoda uno spostamento: nella stessa direzione somma, nell'opposta annulla l'attesa e riparte da `n`. */
export function queueNav(pending: Nav | null, dir: 1 | -1, n: number): { pending: Nav; cancel: boolean } {
  if (!pending || pending.dir === dir) return { pending: { dir, n: (pending?.n ?? 0) + n }, cancel: false };
  return { pending: { dir, n }, cancel: true };
}
