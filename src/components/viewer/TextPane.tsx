import { useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
import { readChunk } from "../../ipc/api";
import { codeUnit } from "../../lib/encoding";
import { lineUp, MAX_LINE, seekLine, splitLines, type Win } from "../../lib/textPaging";
import { useViewers } from "../../state/viewers";
import { usePaneRows, WHEEL_LINES, type PaneHandle } from "./pane";
import { ScrollRail } from "./ScrollRail";

// Il testo non si carica mai intero: una finestra di 256 KiB, di cui 64 KiB prima di `top` perché ↑ non
// rilegga a ogni riga. Salti (↑ oltre la finestra, Fine, barra) leggono un blocco da 64 KiB.
const WINDOW = 256 * 1024;
const HEAD = 64 * 1024;
const BLOCK = 64 * 1024;

interface Layout {
  chunk: Uint8Array;
  /** Inizio di ogni riga del blocco dopo `top`; solo le prime `n` sono complete. */
  starts: number[];
  n: number;
  /** Il blocco arriva a fine file: tutte le righe sono complete. */
  all: boolean;
}

/** Righe della finestra a partire da `top`; null se non bastano `rows + 1` righe complete (serve una nuova lettura). */
export function layoutAt(win: Win | null, top: number, rows: number, unit: 1 | 2, bigEndian: boolean): Layout | null {
  if (!win) return null;
  const rel = top - win.base;
  if (rel < 0 || rel > win.buf.length) return null;
  const chunk = win.buf.subarray(rel, rel + (rows + 2) * MAX_LINE);
  const all = win.eof && rel + chunk.length === win.buf.length;
  const starts = splitLines(chunk, top, unit, bigEndian);
  // L'ultima riga di un blocco che non arriva a fine file può essere tagliata: non si mostra.
  const n = all ? starts.length : starts.length - 1;
  return all || n > rows ? { chunk, starts, n, all } : null;
}

/**
 * Righe logiche che devono restare sotto `top` alla fine del file. A capo una riga logica occupa più righe
 * visive e il viewport è fisso: si ancora l'ultima riga in cima, altrimenti la fine non sarebbe raggiungibile.
 */
export const endRows = (rows: number, wrap: boolean) => (wrap ? 1 : rows);

/** Righe di cui si può scendere da `top`: con la fine del file nel blocco ci si ferma quando l'ultima pagina è piena. */
export const maxScroll = (lay: Pick<Layout, "n" | "all">, rows: number, wrap: boolean) =>
  lay.all ? Math.max(0, lay.n - endRows(rows, wrap)) : lay.n - 1;

export function decoderFor(encoding: string): TextDecoder {
  try {
    return new TextDecoder(encoding);
  } catch {
    return new TextDecoder(); // etichetta sconosciuta: UTF-8
  }
}

interface Props {
  tabId: string;
  path: string;
  size: number;
  top: number;
  encoding: string;
  wrap: boolean;
  onError: (e: unknown) => void;
  ref?: Ref<PaneHandle>;
}

/** Vista testo a righe logiche; ↑ ↓ PgSu PgGiù Home Fine e la barra spostano `top` (byte), mai per pixel. */
export function TextPane({ tabId, path, size, top, encoding, wrap, onError, ref }: Props) {
  const viewport = useRef<HTMLDivElement>(null);
  const rows = usePaneRows(viewport);
  const unit = codeUnit(encoding);
  const bigEndian = encoding === "utf-16be";
  const [win, setWin] = useState<Win | null>(null);
  const winRef = useRef<Win | null>(null);
  // Numeri di sequenza: una risposta di lettura superata da una richiesta più recente si scarta.
  const seq = useRef({ win: 0, op: 0 });
  const latest = useRef({ rows, unit, bigEndian, size, wrap });
  latest.current = { rows, unit, bigEndian, size, wrap };

  const putWin = (w: Win) => {
    winRef.current = w;
    setWin(w);
  };
  const getTop = () => useViewers.getState().states[tabId]?.top ?? 0;
  const setTop = (to: number) => useViewers.getState().set(tabId, { top: to });

  async function loadWindow(at: number): Promise<Win> {
    const { rows: r, unit: u } = latest.current;
    const from = Math.max(0, at - HEAD);
    const base = from - (from % u);
    const len = Math.max(WINDOW, HEAD + (r + 2) * MAX_LINE);
    const buf = await readChunk(path, base, len);
    return { base, buf, eof: buf.length < len };
  }

  const layout = useMemo(() => layoutAt(win, top, rows, unit, bigEndian), [win, top, rows, unit, bigEndian]);

  // Se la finestra non copre `top` (apertura, salto, ridimensionamento) se ne legge una nuova.
  useEffect(() => {
    if (layout || !rows) return;
    const mine = ++seq.current.win;
    loadWindow(top)
      .then((w) => mine === seq.current.win && putWin(w))
      .catch((e) => mine === seq.current.win && onError(e));
  }, [!layout, top, rows, path, unit]);

  // Byte [from, from + len): dalla finestra se ci stanno, altrimenti dal file.
  const cached = (from: number, len: number) => {
    const w = winRef.current;
    return w && from >= w.base && from + len <= w.base + w.buf.length ? w.buf.subarray(from - w.base, from - w.base + len) : null;
  };

  async function down(n: number) {
    const mine = ++seq.current.op;
    const { rows: r, unit: u, bigEndian: be, wrap: w } = latest.current;
    const at = getTop();
    let lay = layoutAt(winRef.current, at, r, u, be);
    if (!lay) {
      const w = await loadWindow(at);
      if (mine !== seq.current.op) return;
      seq.current.win++; // la finestra nuova vale più di una lettura dell'effetto ancora in volo
      putWin(w);
      lay = layoutAt(w, at, r, u, be);
    }
    if (!lay) return;
    const k = Math.min(n, maxScroll(lay, r, w));
    if (k > 0) setTop(lay.starts[k]);
  }

  async function up(n: number) {
    const mine = ++seq.current.op;
    const { unit: u, bigEndian: be } = latest.current;
    const at = getTop();
    if (at <= 0) return;
    const len = Math.min(at, Math.max(BLOCK, (n + 1) * MAX_LINE));
    const base = at - len;
    let before = cached(base, len);
    if (!before) {
      before = await readChunk(path, base, len);
      if (mine !== seq.current.op) return;
    }
    if (before.length < len) return; // il file è cambiato sotto di noi
    let t = at;
    for (let i = 0; i < n && t > base; i++) {
      const prev = lineUp(before.subarray(0, t - base), base, t, u, be);
      if (prev >= t) break;
      t = prev;
    }
    if (t < at) setTop(t);
  }

  async function toEnd() {
    const mine = ++seq.current.op;
    const { rows: r, unit: u, bigEndian: be, size: total, wrap: w } = latest.current;
    const stop = total - (total % u);
    if (stop <= 0) return setTop(0);
    const len = Math.min(stop, Math.max(BLOCK, (r + 1) * MAX_LINE));
    const base = stop - len;
    const buf = await readChunk(path, base, len);
    if (mine !== seq.current.op) return;
    let t = stop;
    for (let i = 0; i < endRows(r, w) && t > base; i++) {
      const prev = lineUp(buf.subarray(0, t - base), base, t, u, be);
      if (prev >= t) break;
      t = prev;
    }
    setTop(t);
  }

  // Dalla barra: il primo inizio riga dopo l'offset puntato.
  async function seek(fraction: number) {
    const mine = ++seq.current.op;
    const { unit: u, bigEndian: be, size: total } = latest.current;
    const raw = Math.min(total, Math.floor(fraction * total));
    const offset = raw - (raw % u);
    const chunk = await readChunk(path, offset, BLOCK);
    if (mine !== seq.current.op) return;
    const t = seekLine(chunk, offset, u, be);
    if (t >= total) return toEnd();
    setTop(t);
  }

  const run = (p: Promise<void>) => p.catch(onError);
  const scroll = (lines: number) => {
    if (lines > 0) run(down(lines));
    else if (lines < 0) run(up(-lines));
  };

  useImperativeHandle(ref, () => ({
    scroll,
    page: (dir) => scroll(dir * Math.max(1, rows - 1)),
    edge: (end) => {
      if (end) run(toEnd());
      else {
        seq.current.op++; // annulla un salto in corso
        setTop(0);
      }
    },
  }));

  // Si decodificano solo le righe visibili più una.
  const lines = useMemo(() => {
    if (!layout) return [];
    const decoder = decoderFor(encoding);
    const { chunk, starts, n } = layout;
    return starts.slice(0, Math.min(n, rows + 1)).map((s, i) => {
      const end = i + 1 < starts.length ? starts[i + 1] : top + chunk.length;
      return decoder.decode(chunk.subarray(s - top, end - top)).replace(/\r?\n$/, "");
    });
  }, [layout, encoding, rows, top]);

  return (
    <div className="mc-vpane">
      <div
        ref={viewport}
        className={`mc-vtext${wrap ? " wrap" : ""}`}
        onWheel={(e) => e.deltaY && scroll(Math.sign(e.deltaY) * WHEEL_LINES)}
      >
        {lines.map((l, i) => <div key={i} className="mc-vline">{l}</div>)}
      </div>
      <ScrollRail fraction={size ? top / size : 0} onSeek={(f) => run(seek(f))} />
    </div>
  );
}
