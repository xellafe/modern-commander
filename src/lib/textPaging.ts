// Righe a blocchi: il viewer legge finestre di byte, non il file intero. Una riga più lunga di MAX_LINE
// (log da 5 GB, JSON minificato su una riga, file di zeri) si spezza in righe da MAX_LINE byte, così
// ogni passo di scorrimento resta limitato. Gli offset sono sempre multipli dell'unità di codice.

/** Byte; una riga più lunga si spezza in righe da MAX_LINE. */
export const MAX_LINE = 4096;

// LF all'indice `i`: `0A` (unit 1), `0A 00` (UTF-16LE) o `00 0A` (UTF-16BE).
function isLf(buf: Uint8Array, i: number, unit: 1 | 2, bigEndian: boolean): boolean {
  if (unit === 1) return buf[i] === 0x0a;
  return bigEndian ? buf[i] === 0 && buf[i + 1] === 0x0a : buf[i] === 0x0a && buf[i + 1] === 0;
}

/** Offset assoluto di ogni inizio riga nel blocco (che parte da `base`, a inizio riga), solo quelli dentro il blocco. */
export function splitLines(chunk: Uint8Array, base: number, unit: 1 | 2, bigEndian: boolean): number[] {
  const starts: number[] = [];
  let pos = 0;
  while (pos < chunk.length) {
    starts.push(base + pos);
    const limit = Math.min(pos + MAX_LINE, chunk.length);
    let next = pos + MAX_LINE;
    for (let i = pos; i + unit <= limit; i += unit) {
      if (isLf(chunk, i, unit, bigEndian)) { next = i + unit; break; }
    }
    pos = next;
  }
  return starts;
}

/** Inizio della riga sopra `top`; `before` è il blocco da `base` a `top`. */
export function lineUp(before: Uint8Array, base: number, top: number, unit: 1 | 2, bigEndian: boolean): number {
  // L'ultima unità è il terminatore della riga sopra (o un suo pezzo): si cerca l'LF prima.
  let start = -1;
  for (let i = before.length - 2 * unit; i >= 0; i -= unit) {
    if (isLf(before, i, unit, bigEndian)) { start = base + i + unit; break; }
  }
  if (start < 0) {
    // Senza LF l'inizio vero è ignoto: a base 0 la riga parte da 0, altrimenti si risale di MAX_LINE.
    if (base > 0) return Math.max(base, top - MAX_LINE);
    start = base;
  }
  if (top <= start) return start;
  // Riga lunga: l'ultimo spezzone da MAX_LINE che inizia prima di `top`.
  return start + Math.floor((top - 1 - start) / MAX_LINE) * MAX_LINE;
}

/** Salto con la barra di scorrimento: primo inizio riga ≥ `offset` nel blocco che parte da `offset`. */
export function seekLine(chunk: Uint8Array, offset: number, unit: 1 | 2, bigEndian: boolean): number {
  if (offset === 0) return 0;
  const skip = (unit - (offset % unit)) % unit; // byte fino al primo confine di unità
  const limit = Math.min(MAX_LINE, chunk.length);
  for (let i = skip; i + unit <= limit; i += unit) {
    if (isLf(chunk, i, unit, bigEndian)) return offset + i + unit;
  }
  return offset - (offset % unit);
}
