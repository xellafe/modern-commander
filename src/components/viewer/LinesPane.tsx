import { useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
import { cancelRequest, columnOffsets, LINE_END, lineStart, nextLineStart, nextRequestId, readChunk } from "../../ipc/api";
import type { AppError, ColumnHit, ColumnProgress } from "../../ipc/types";
import { codeUnit, detectEncoding } from "../../lib/encoding";
import {
  columnHits, lastPageTop, lineAt, lineExtent, maxLeft, nextStart, queueNav, renderSlice, stepDown, stepUp, visibleLines,
  type Columns, type Line, type Nav, type NavCtx, type Scans,
} from "../../lib/longLines";
import type { Win } from "../../lib/textPaging";
import { lineCache } from "../../state/lineCache";
import { useViewers } from "../../state/viewers";
import { usePaneCols, usePaneRows, WHEEL_COLS, WHEEL_LINES, type PaneHandle } from "./pane";
import { ScrollRail } from "./ScrollRail";
import { decoderFor } from "./TextPane";

// Finestra come in TextPane: 256 KiB, di cui 64 KiB prima di `top`. Le righe che ne escono le risolve il backend.
const WINDOW = 256 * 1024;
const HEAD = 64 * 1024;

interface Props {
  tabId: string;
  path: string;
  size: number;
  top: number;
  /** Colonna del primo carattere visibile. */
  left: number;
  encoding: string;
  /** Avanzamento delle scansioni (0…1), null quando non ce ne sono. */
  onBusy: (fraction: number | null) => void;
  onError: (e: unknown) => void;
  ref?: Ref<PaneHandle>;
}

// I byte da `from` a `stop` se la finestra li contiene (a fine file fin dove arriva).
function fromWindow(w: Win | null, from: number, stop: number): Uint8Array | null {
  if (!w || from < w.base || from > w.base + w.buf.length) return null;
  return stop <= w.base + w.buf.length || w.eof ? w.buf.subarray(from - w.base, stop - w.base) : null;
}

/**
 * Ciò che si mostra delle righe con la colonna `left`: il carattere da cui parte ogni riga e i byte letti fuori dalla
 * finestra (per offset). Si sostituisce intero quando è pronto: fino ad allora resta quello di prima, senza righe in
 * sospeso che lampeggiano a ogni → e senza righe a colonne diverse.
 */
interface Frame {
  left: number;
  encoding: string;
  /** Byte letti per riga: le colonne visibili più una, 4 byte ciascuna. */
  need: number;
  hits: Map<number, ColumnHit>;
  chunks: Map<number, Uint8Array>;
}

// Con `left = 0` il carattere è l'inizio riga, senza backend.
const zero = (l: Line): ColumnHit => ({ offset: l.start, column: 0, eol: false });
// Fin dove leggere la riga dal carattere `h`: `need` byte, mai oltre la fine nota.
const stopOf = (l: Line, h: ColumnHit, need: number) => Math.min(l.end ?? Infinity, h.offset + need);

/** Vista testo con l'a capo spento: una riga logica è una riga a schermo, qualunque sia la sua lunghezza. */
export function LinesPane({ tabId, path, size, top, left, encoding, onBusy, onError, ref }: Props) {
  const viewport = useRef<HTMLDivElement>(null);
  const column = useRef<HTMLDivElement>(null);
  const rows = usePaneRows(viewport);
  // Righe sull'altezza piena, barra orizzontale compresa: le barre si decidono su queste e non oscillano.
  const allRows = usePaneRows(column);
  const cols = usePaneCols(viewport);
  const unit = codeUnit(encoding);
  const bigEndian = encoding === "utf-16be";
  const [win, setWin] = useState<Win | null>(null);
  const winRef = useRef<Win | null>(null);
  const [frame, setFrame] = useState<Frame>({ left: 0, encoding: "", need: 0, hits: new Map(), chunks: new Map() });
  // Il caricamento del frame in corso: chiave (per non rilanciarlo uguale), numero di sequenza e richiesta delle colonne.
  // `stopped`: il `tick` dell'ultimo annullamento; finché non cambia (una navigazione) il frame non si ricarica.
  const framing = useRef<{ key: string | null; seq: number; req: number | null; stopped: number }>({ key: null, seq: 0, req: null, stopped: -1 });
  // ← → che aspettano il conteggio delle larghezze: i passi nella stessa direzione si sommano.
  const hop = useRef<{ delta: number; id: number | null } | null>(null);
  // La cache delle righe non è reattiva: `tick` ridisegna.
  const [tick, setTick] = useState(0);
  const tickRef = useRef(tick);
  tickRef.current = tick;
  const loading = useRef<{ base: number; p: Promise<Win> } | null>(null);
  const bom = useRef<{ key: string; p: Promise<number>; n: number } | null>(null);
  // `top` già riportato a un inizio riga vero, con la codifica per cui lo è.
  const valid = useRef<{ top: number; encoding: string } | null>(null);
  const active = useRef(new Map<number, { total: number; done: number }>());
  const inflight = useRef(new Map<number, Promise<number>>());
  const nav = useRef<Nav | null>(null);
  // La normalizzazione in corso di `top`: i tasti premuti nel frattempo non ne lanciano altre.
  const fixing = useRef<{ key: string; p: Promise<void> } | null>(null);
  // Cresce a ogni annullamento: un passo o un salto superato non sposta più `top`.
  const gen = useRef(0);
  const latest = useRef({ rows, cols, unit, bigEndian, size, encoding, onBusy, onError });
  latest.current = { rows, cols, unit, bigEndian, size, encoding, onBusy, onError };
  const linesRef = useRef<Line[]>([]);
  const allRef = useRef<Line[]>([]);

  const getTop = () => useViewers.getState().states[tabId]?.top ?? 0;
  const setTop = (to: number) => {
    valid.current = { top: to, encoding: latest.current.encoding };
    useViewers.getState().set(tabId, { top: to });
    setTick((n) => n + 1); // anche senza spostamento: le righe in sospeso si richiedono di nuovo
  };
  const getLeft = () => useViewers.getState().states[tabId]?.left ?? 0;
  const setLeft = (to: number) => {
    useViewers.getState().set(tabId, { left: to });
    setTick((n) => n + 1);
  };
  const fail = (e: unknown) => {
    if ((e as Partial<AppError> | null)?.code !== "CANCELLED") latest.current.onError(e);
  };
  const run = (p: Promise<unknown>) => p.catch(fail);

  // Avanzamento: byte fatti su byte da fare, sommati sulle scansioni attive.
  const busy = () => {
    let done = 0;
    let total = 0;
    for (const r of active.current.values()) {
      done += r.done;
      total += r.total;
    }
    latest.current.onBusy(active.current.size ? (total ? done / total : 0) : null);
  };
  function track<P, T>(total: number, done: (p: P) => number, onProgress: (p: P) => void, call: (id: number, progress: (p: P) => void) => Promise<T>) {
    const id = nextRequestId();
    const r = { total, done: 0 };
    active.current.set(id, r);
    busy();
    return call(id, (o) => {
      r.done = done(o);
      onProgress(o);
      busy();
    }).finally(() => {
      active.current.delete(id);
      busy();
    });
  }
  const scans: Scans = {
    next: (from, onProgress) => {
      const { size: s, unit: u, bigEndian: be } = latest.current;
      return track(s - from, (o) => o - from, onProgress, (id, p) => nextLineStart(id, path, from, u, be, p));
    },
    start: (at, onProgress) => {
      const { unit: u, bigEndian: be } = latest.current;
      return track(at, (o) => at - o, onProgress, (id, p) => lineStart(id, path, at, u, be, p));
    },
  };

  // Colonne dal backend per la codifica `enc`; `onId` riceve il numero della richiesta, per annullarla se superata.
  // Avanzamento: byte contati su byte fino alla fine (nota, o del file) di ogni riga.
  const columns = (enc: string, onId?: (id: number) => void): Columns => (ls, from, target, onProgress) => {
    const s = latest.current.size;
    const done = from.map(() => 0);
    const total = ls.reduce((t, l, i) => t + (l.end ?? s) - from[i].offset, 0);
    return track(
      total,
      (p: ColumnProgress) => {
        done[p.index] = p.mark.offset - from[p.index].offset;
        return done.reduce((a, b) => a + b, 0);
      },
      onProgress,
      (id, p) => {
        onId?.(id);
        return columnOffsets(id, path, enc, from, target, p);
      },
    );
  };

  // Il BOM non si mostra mai: la prima riga inizia dopo, se è quello della codifica scelta.
  function getBom(): Promise<number> {
    const enc = latest.current.encoding;
    const key = `${path}|${enc}`;
    if (bom.current?.key !== key) {
      const b = { key, n: 0, p: Promise.resolve(0) };
      b.p = readChunk(path, 0, 3).then((head) => {
        const d = detectEncoding(head, enc, true);
        return (b.n = d.encoding === enc ? d.bom : 0);
      });
      bom.current = b;
    }
    return bom.current.p;
  }

  const ctx = (w = winRef.current): NavCtx => {
    const l = latest.current;
    return { win: w, cache: lineCache(tabId, l.encoding), unit: l.unit, bigEndian: l.bigEndian, size: l.size, bom: bom.current?.n ?? 0, scans, inflight: inflight.current };
  };

  // Una finestra nuova serve se non copre `at`, se è allineata a un'altra unità, se resta poca storia per ↑, o se
  // una finestra da `at - HEAD` vedrebbe righe intere che questa taglia.
  function stale(w: Win | null, at: number): boolean {
    const { rows: r, unit: u } = latest.current;
    if (!w || at < w.base || at > w.base + w.buf.length || w.base % u) return true;
    if (w.base > 0 && at - w.base < HEAD / 2) return true;
    if (w.eof || at - w.base <= HEAD) return false;
    const ls = visibleLines(ctx(w), at, r + 1);
    return ls[ls.length - 1].next === null;
  }

  // La finestra per `at`: quella attuale se basta, altrimenti una lettura (una sola per base anche se chiesta più volte).
  function ensureWindow(at: number): Promise<Win> {
    const w = winRef.current;
    if (!stale(w, at)) return Promise.resolve(w!);
    const from = Math.max(0, at - HEAD);
    const base = from - (from % latest.current.unit);
    if (loading.current?.base !== base) {
      const p: Promise<Win> = readChunk(path, base, WINDOW)
        .then((buf) => {
          const nw = { base, buf, eof: buf.length < WINDOW };
          if (loading.current?.p === p) {
            winRef.current = nw;
            setWin(nw);
          }
          return nw;
        })
        .finally(() => {
          if (loading.current?.p === p) loading.current = null;
        });
      loading.current = { base, p };
    }
    return loading.current.p;
  }

  // Dopo un annullamento o un errore il frame di un'altra `left` non resta: al suo posto i segnaposto, che una nuova
  // navigazione richiede di nuovo.
  function drop() {
    const l = getLeft();
    const enc = latest.current.encoding;
    framing.current.stopped = tickRef.current;
    setFrame((f) => (f.left === l && f.encoding === enc ? f : { left: l, encoding: enc, need: 0, hits: new Map(), chunks: new Map() }));
  }

  // Con `stop` falso il frame mostrato resta: smontando, cambiando codifica (lì si ricarica comunque) e trascinando la
  // barra orizzontale (il `setLeft` che segue lo ricarica).
  function cancel(stop = true): boolean {
    gen.current++;
    nav.current = null;
    hop.current = null;
    inflight.current.clear();
    const fr = framing.current;
    fr.seq++;
    fr.key = null;
    if (stop) drop();
    const ids = [...active.current.keys()];
    for (const id of ids) run(cancelRequest(id));
    return ids.length > 0;
  }

  // Arrivando dall'a capo attivo (o da un'altra codifica) `top` può cadere a metà riga: si riporta all'inizio.
  // Una sola per `top` e codifica; dopo un annullamento (`gen` cambia) se ne lancia una nuova.
  function normalize(): Promise<void> {
    const g = gen.current;
    const enc = latest.current.encoding;
    const t = getTop();
    const key = `${t}|${enc}|${g}`;
    if (fixing.current?.key !== key) {
      const p: Promise<void> = (async () => {
        await getBom();
        const s = await lineAt(ctx(await ensureWindow(t)), t);
        if (g === gen.current && latest.current.encoding === enc && getTop() === t) setTop(s);
      })().finally(() => {
        if (fixing.current?.p === p) fixing.current = null;
      });
      fixing.current = { key, p };
    }
    return fixing.current.p;
  }

  // Passi accodati: si avanza riga per riga, aspettando il backend quando una riga esce dalla finestra.
  // React ridisegna tra un `await` e l'altro: i passi risolti dalla finestra si mostrano solo all'ultimo (o ogni 16 ms),
  // quelli che aspettano il backend man mano.
  async function drain() {
    const g = gen.current;
    let at = getTop();
    let shown = performance.now();
    // La finestra prima di `top`: nessun fotogramma con le righe in sospeso.
    const show = async () => {
      await ensureWindow(at);
      if (g !== gen.current) return false;
      setTop(at);
      shown = performance.now();
      return true;
    };
    try {
      while (nav.current && nav.current.n > 0) {
        const w = await ensureWindow(at);
        if (g !== gen.current) return;
        const t = nav.current.dir > 0 ? await stepDown(ctx(w), at, latest.current.rows) : await stepUp(ctx(w), at);
        if (g !== gen.current) return;
        if (t === null) {
          if (at !== getTop()) await show();
          return;
        }
        at = t;
        nav.current.n--;
        if ((nav.current.n === 0 || performance.now() - shown > 16) && !(await show())) return;
      }
    } finally {
      if (g === gen.current) {
        nav.current = null;
        // Anche se `top` non è cambiato: una navigazione richiede di nuovo le righe in sospeso.
        setTick((n) => n + 1);
      }
    }
  }

  async function toEnd(g: number) {
    await getBom();
    const { size: s, unit: u, rows: r } = latest.current;
    const t = await lastPageTop(ctx(await ensureWindow(s - (s % u))), r);
    if (g === gen.current) setTop(t);
  }

  // Dalla barra: l'inizio della riga che contiene l'offset puntato.
  async function seek(fraction: number, g: number) {
    await getBom();
    const { size: s, unit: u } = latest.current;
    const raw = Math.min(s, Math.floor(fraction * s));
    const at = raw - (raw % u);
    const t = await lineAt(ctx(await ensureWindow(at)), at);
    if (g !== gen.current) return;
    if (t >= s) return toEnd(g);
    setTop(t);
  }

  // Estensione orizzontale sulle righe dell'altezza piena: il conteggio esatto se noto, altrimenti la stima in byte.
  const extentNow = () => {
    const l = latest.current;
    return lineExtent(allRef.current, lineCache(tabId, l.encoding), l.size, l.unit);
  };

  // Un → in attesa del conteggio non vale più: ← o un salto lo sostituiscono.
  function stopHop() {
    const id = hop.current?.id;
    hop.current = null;
    if (id != null && active.current.has(id)) run(cancelRequest(id));
  }

  // ← →: `left` subito, limitato all'estensione. La stima in byte non conta i tab: oltre il limite stimato si contano
  // prima le righe senza larghezza nota (corte, se il limite è vicino), poi si limita col conteggio esatto. I → premuti
  // nel frattempo si sommano e si applicano alla `left` di quel momento.
  function hscroll(n: number) {
    if (!n || !ready) return;
    if (hop.current && n > 0) return void (hop.current.delta += n);
    stopHop();
    const max = () => maxLeft(extentNow(), latest.current.cols);
    const to = Math.max(0, getLeft() + n);
    const cache = lineCache(tabId, latest.current.encoding);
    const unknown = allRef.current.filter((l) => !cache.widths.has(l.start));
    if (n < 0 || to <= max() || !unknown.length) return setLeft(Math.min(to, max()));
    const op: { delta: number; id: number | null } = { delta: n, id: null };
    hop.current = op;
    run(
      columnHits(cache, unknown, LINE_END, columns(cache.encoding, (id) => (op.id = id)))
        .then(() => {
          if (hop.current === op) setLeft(Math.min(Math.max(0, getLeft() + op.delta), max()));
        })
        .finally(() => {
          if (hop.current === op) hop.current = null;
        }),
    );
  }

  // Home / Fine: inizio riga, oppure la fine della riga più lunga (contando quelle senza larghezza nota), sulle stesse
  // righe dell'estensione.
  function hedge(end: boolean) {
    cancel();
    if (!end) return setLeft(0);
    const ls = allRef.current;
    if (!ls.length) return;
    const g = gen.current;
    const enc = latest.current.encoding;
    run(columnHits(lineCache(tabId, enc), ls, LINE_END, columns(enc)).then((hits) => {
      if (g === gen.current) setLeft(maxLeft(Math.max(...hits.map((h) => h.column)), latest.current.cols));
    }));
  }

  // Il frame per `to` sulle righe `ls`: le colonne che mancano in una richiesta sola, poi i byte fuori dalla finestra
  // tutti insieme. Si riusa ciò che il frame di prima ha già.
  async function load(ls: Line[], to: number, enc: string, need: number, base: Frame): Promise<Frame> {
    const same = base.encoding === enc;
    const hits = new Map<number, ColumnHit>();
    for (const l of ls) {
      const h = !to ? zero(l) : same && base.left === to ? base.hits.get(l.start) : undefined;
      if (h) hits.set(l.start, h);
    }
    const ask = ls.filter((l) => !hits.has(l.start));
    if (ask.length) {
      const got = await columnHits(lineCache(tabId, enc), ask, to, columns(enc, (id) => (framing.current.req = id)));
      ask.forEach((l, i) => hits.set(l.start, got[i]));
    }
    const chunks = new Map<number, Uint8Array>();
    const reads: [number, number][] = [];
    for (const l of ls) {
      const h = hits.get(l.start)!;
      const stop = stopOf(l, h, need);
      if (h.eol || fromWindow(winRef.current, h.offset, stop)) continue;
      const c = same && base.need >= need ? base.chunks.get(h.offset) : undefined;
      if (c) chunks.set(h.offset, c);
      else reads.push([h.offset, stop - h.offset]);
    }
    const bufs = await Promise.all(reads.map(([o, n]) => readChunk(path, o, n)));
    reads.forEach(([o], i) => chunks.set(o, bufs[i]));
    return { left: to, encoding: enc, need, hits, chunks };
  }

  const ready = valid.current?.top === top && valid.current.encoding === encoding;

  const scroll = (lines: number) => {
    if (!lines) return;
    if (!ready) return void run(normalize()); // dopo un annullamento: si riprova
    const { pending, cancel: opposite } = queueNav(nav.current, lines > 0 ? 1 : -1, Math.abs(lines));
    const running = nav.current !== null && !opposite;
    if (opposite) cancel();
    nav.current = pending;
    if (!running) run(drain());
  };

  useImperativeHandle(ref, () => ({
    scroll,
    page: (dir) => scroll(dir * Math.max(1, rows - 1)),
    edge: (end) => {
      cancel();
      const g = gen.current;
      run(end ? toEnd(g) : getBom().then((b) => g === gen.current && setTop(b)));
    },
    hscroll,
    hpage: (dir) => hscroll(dir * Math.max(1, cols - 1)),
    hedge,
    cancel,
  }));

  // Cambiando codifica o smontando (cambio scheda, chiusura) si annulla tutto.
  useEffect(() => () => void cancel(false), [path, encoding]);

  useEffect(() => {
    if (!ready) run(normalize());
  }, [ready, top, encoding]);

  const all = useMemo(
    () => (ready && rows ? visibleLines(ctx(win), top, Math.max(rows, allRows)) : []),
    [ready, win, top, rows, allRows, tick],
  );
  const lines = useMemo(() => all.slice(0, rows), [all, rows]);
  allRef.current = all;
  linesRef.current = lines;

  // Finestra da rileggere, oppure la riga in fondo senza fine nota: la cerca il backend da solo.
  useEffect(() => {
    if (!lines.length) return;
    if (stale(winRef.current, top)) return void run(ensureWindow(top));
    const last = lines[lines.length - 1];
    if (last.next === null) run(nextStart(ctx(), last.start).then(() => setTick((n) => n + 1)));
  }, [lines]);

  // Il frame per la `left` attuale, se quello mostrato non basta. Uno superato si scarta e la sua richiesta si annulla.
  useEffect(() => {
    if (!lines.length || !cols) return;
    const need = (cols + 1) * 4;
    const f = frame;
    const complete = f.encoding === encoding && f.left === left && f.need >= need && lines.every((l) => {
      const h = left ? f.hits.get(l.start) : zero(l);
      return h && (h.eol || fromWindow(winRef.current, h.offset, stopOf(l, h, need)) || f.chunks.has(h.offset));
    });
    const key = `${left}|${encoding}|${need}|${lines.map((l) => l.start).join(",")}`;
    const fr = framing.current;
    if (complete || fr.key === key || fr.stopped === tick) return;
    if (fr.req !== null && active.current.has(fr.req)) run(cancelRequest(fr.req));
    const mine = ++fr.seq;
    fr.key = key;
    run(
      load(lines, left, encoding, need, f)
        .then(
          (nf) => {
            if (mine === fr.seq) setFrame(nf);
          },
          (e) => {
            if (mine === fr.seq) drop();
            throw e;
          },
        )
        .finally(() => {
          if (fr.key === key) fr.key = null;
        }),
    );
  }, [lines, left, cols, encoding, frame, win, tick]);

  // Si mostra il frame più recente: mentre quello per la `left` attuale si carica resta il precedente.
  const texts = useMemo(() => {
    const f = frame.encoding === encoding ? frame : { ...frame, left, hits: new Map<number, ColumnHit>(), chunks: new Map<number, Uint8Array>() };
    const need = (cols + 1) * 4;
    const decoder = decoderFor(encoding);
    return lines.map((l) => {
      const h = f.left ? f.hits.get(l.start) : zero(l);
      if (!h) return null;
      if (h.eol) return ""; // più corta di `left`
      const bytes = fromWindow(win, h.offset, stopOf(l, h, need)) ?? f.chunks.get(h.offset);
      if (!bytes) return null;
      // Letta oltre la fine nota: si taglia al primo LF (e al CR prima).
      const t = decoder.decode(bytes);
      const i = t.indexOf("\n");
      return renderSlice(i < 0 ? t : t.slice(0, i).replace(/\r$/, ""), h.column, f.left, cols);
    });
  }, [lines, frame, win, encoding, left, cols]);

  const last = lines[lines.length - 1];
  // Dopo una riga senza fine nota le righe sono in sospeso.
  const pending = last && last.next === null ? rows - lines.length : 0;
  // Niente da scorrere in verticale: si è in cima e la riga che finisce a fine file è visibile.
  const fits = ready && top <= (bom.current?.n ?? 0) && last?.next != null && last.next >= size;
  const extent = lineExtent(all, lineCache(tabId, encoding), size, unit);
  const maxL = maxLeft(extent, cols);

  return (
    <div className="mc-vpane">
      <div ref={column} className="mc-vcol">
        <div
          ref={viewport}
          className="mc-vtext lines"
          onWheel={(e) => {
            const dx = e.deltaX || (e.shiftKey ? e.deltaY : 0);
            if (dx) hscroll(Math.sign(dx) * WHEEL_COLS);
            else if (e.deltaY) scroll(Math.sign(e.deltaY) * WHEEL_LINES);
          }}
        >
          {texts.map((t, i) => (t === null ? <div key={i} className="mc-vline pending">…</div> : <div key={i} className="mc-vline">{t}</div>))}
          {Array.from({ length: Math.max(0, pending) }, (_, i) => <div key={`p${i}`} className="mc-vline pending">…</div>)}
        </div>
        {cols > 0 && extent > cols && (
          // Annulla la navigazione in corso ma tiene il frame di prima: trascinando, niente segnaposto a ogni movimento.
          <ScrollRail orientation="horizontal" fraction={left / Math.max(1, maxL)} onSeek={(f) => {
            cancel(false);
            setLeft(Math.round(f * maxL));
          }} />
        )}
      </div>
      {!fits && <ScrollRail fraction={size ? top / size : 0} onSeek={(f) => {
        cancel();
        run(seek(f, gen.current));
      }} />}
    </div>
  );
}
