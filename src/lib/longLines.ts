// Righe intere nel viewer: a differenza di textPaging, qui una riga non si spezza a MAX_LINE. Le funzioni
// lavorano sulla finestra di byte già letta; ciò che non sta nella finestra resta ignoto (null) e lo
// risolve il backend (ricerca degli inizi riga e conteggio delle colonne a blocchi).
import type { Mark } from "../ipc/types";
import type { LineCache } from "../state/lineCache";
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

/** Scansioni del backend (`nextLineStart`, `lineStart`), iniettate perché la navigazione si provi senza Tauri. */
export interface Scans {
  next(from: number, onProgress: (offset: number) => void): Promise<number>;
  start(at: number, onProgress: (offset: number) => void): Promise<number>;
}

/** Ciò che serve alla navigazione verticale per righe intere: finestra, cache della scheda e backend. */
export interface NavCtx {
  win: Win | null;
  cache: LineCache;
  unit: 1 | 2;
  bigEndian: boolean;
  size: number;
  /** Byte del BOM: la prima riga inizia lì e `top` non scende sotto. */
  bom: number;
  scans: Scans;
  /** Scansioni in avanti in corso per inizio riga: due richieste per la stessa riga ne fanno una. */
  inflight: Map<number, Promise<number>>;
}

// Una risposta arrivata dopo un cambio di codifica non scrive nella cache, che nel frattempo è stata reimpostata.
const sameEncoding = (cache: LineCache, encoding: string) => cache.encoding === encoding;

/**
 * Righe visibili da `top`, al massimo `max`: dalla finestra, poi da `next` in cache. Si ferma alla prima riga di cui
 * non si conosce l'inizio della successiva; la riga vuota dopo l'ultimo LF non c'è.
 */
export function visibleLines(ctx: NavCtx, top: number, max: number): Line[] {
  const lines = ctx.win ? wholeLines(ctx.win, top, max, ctx.unit, ctx.bigEndian) : [];
  if (!lines.length) lines.push({ start: top, end: null, next: null });
  for (;;) {
    const last = lines[lines.length - 1];
    last.next ??= ctx.cache.next.get(last.start) ?? null;
    if (last.next === null || last.next >= ctx.size || lines.length >= max) break;
    lines.push({ start: last.next, end: null, next: null });
  }
  return lines.filter((l, i) => i === 0 || l.start < ctx.size);
}

/** Inizio della riga dopo quella che parte da `s`: dalla finestra, dalla cache o dal backend (che aggiorna `fwd` e `next`). */
export function nextStart(ctx: NavCtx, s: number): Promise<number> {
  const known = visibleLines(ctx, s, 1)[0].next;
  if (known !== null) return Promise.resolve(known);
  const running = ctx.inflight.get(s);
  if (running) return running;
  const { win, cache } = ctx;
  const encoding = cache.encoding;
  // Nella finestra non c'è LF da `s` alla sua fine: si cerca da lì, o dal punto più avanzato già noto.
  const winEnd = win && s >= win.base ? win.base + win.buf.length : s;
  const from = cache.fwd.get(s) ?? Math.max(s, winEnd);
  const p = ctx.scans
    .next(from, (o) => sameEncoding(cache, encoding) && cache.fwd.set(s, o))
    .then((r) => {
      if (sameEncoding(cache, encoding)) cache.next.set(s, r);
      return r;
    })
    .finally(() => ctx.inflight.delete(s));
  ctx.inflight.set(s, p);
  return p;
}

/** `top` della riga sotto; null se l'ultima riga del file è già l'ultima del viewport (come `maxScroll` senza a capo). */
export async function stepDown(ctx: NavCtx, top: number, rows: number): Promise<number | null> {
  let s = top;
  let first = top;
  for (let i = 0; i < rows; i++) {
    s = await nextStart(ctx, s);
    if (s >= ctx.size) return null;
    if (i === 0) first = s;
  }
  return first;
}

/** `top` della riga sopra; null alla prima riga. Senza finestra né cache, `lineStart` dal backend (che aggiorna `back`). */
export async function stepUp(ctx: NavCtx, top: number): Promise<number | null> {
  if (top <= ctx.bom) return null;
  const { win, cache } = ctx;
  let p = win ? prevLineStart(win, top, ctx.unit, ctx.bigEndian) : null;
  // ponytail: ricerca lineare di chi ha `top` come successiva; la mappa contiene solo righe passate dal backend.
  if (p === null) for (const [s, n] of cache.next) if (n === top) p = s;
  if (p === null) {
    const encoding = cache.encoding;
    p = await ctx.scans.start(cache.back.get(top) ?? top - ctx.unit, (o) => sameEncoding(cache, encoding) && cache.back.set(top, o));
    if (sameEncoding(cache, encoding)) cache.next.set(p, top);
  }
  return Math.max(p, ctx.bom);
}

/** Inizio della riga che contiene `at`, mai prima del BOM: dalla finestra se basta, altrimenti dal backend. */
export async function lineAt(ctx: NavCtx, at: number): Promise<number> {
  if (at <= ctx.bom) return ctx.bom;
  const here = ctx.win ? isLineStart(ctx.win, at, ctx.unit, ctx.bigEndian) : null;
  if (here) return at;
  // A metà riga l'unità prima di `at` non è un LF: `prevLineStart` trova l'inizio della riga stessa.
  const s = (here === false ? prevLineStart(ctx.win!, at, ctx.unit, ctx.bigEndian) : null) ?? (await ctx.scans.start(at, () => {}));
  return Math.max(s, ctx.bom);
}

/** Ctrl+Fine: risale di `rows` righe dalla fine del file, come `endRows(rows, false)` di `TextPane`. */
export async function lastPageTop(ctx: NavCtx, rows: number): Promise<number> {
  let t = ctx.size - (ctx.size % ctx.unit);
  for (let i = 0; i < rows; i++) {
    const p = await stepUp(ctx, t);
    if (p === null) break;
    t = p;
  }
  return Math.max(t, ctx.bom);
}
