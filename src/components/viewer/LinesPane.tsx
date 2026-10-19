import { useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
import { cancelRequest, lineStart, nextLineStart, nextRequestId, readChunk } from "../../ipc/api";
import type { AppError } from "../../ipc/types";
import { codeUnit, detectEncoding } from "../../lib/encoding";
import {
  lastPageTop, lineAt, nextStart, queueNav, renderSlice, stepDown, stepUp, visibleLines,
  type Line, type Nav, type NavCtx, type Scans,
} from "../../lib/longLines";
import type { Win } from "../../lib/textPaging";
import { lineCache } from "../../state/lineCache";
import { useViewers } from "../../state/viewers";
import { usePaneRows, WHEEL_LINES, type PaneHandle } from "./pane";
import { ScrollRail } from "./ScrollRail";
import { decoderFor } from "./TextPane";

// Finestra come in TextPane: 256 KiB, di cui 64 KiB prima di `top`. Le righe che ne escono le risolve il backend.
const WINDOW = 256 * 1024;
const HEAD = 64 * 1024;
/** Byte letti per riga: bastano al viewport senza scorrimento orizzontale. */
const BYTES = 4096;

interface Props {
  tabId: string;
  path: string;
  size: number;
  top: number;
  /** Colonna del primo carattere visibile (Task 5). */
  left: number;
  encoding: string;
  /** Avanzamento delle scansioni (0…1), null quando non ce ne sono. */
  onBusy: (fraction: number | null) => void;
  onError: (e: unknown) => void;
  ref?: Ref<PaneHandle>;
}

// Byte del testo della riga `l` se la finestra li contiene: fino alla fine nota, al massimo BYTES.
function fromWindow(w: Win | null, l: Line): Uint8Array | null {
  if (!w || l.start < w.base) return null;
  const stop = Math.min(l.end ?? Infinity, l.start + BYTES);
  return stop <= w.base + w.buf.length || w.eof ? w.buf.subarray(l.start - w.base, stop - w.base) : null;
}

/** Vista testo con l'a capo spento: una riga logica è una riga a schermo, qualunque sia la sua lunghezza. */
export function LinesPane({ tabId, path, size, top, encoding, onBusy, onError, ref }: Props) {
  const viewport = useRef<HTMLDivElement>(null);
  const rows = usePaneRows(viewport);
  const unit = codeUnit(encoding);
  const bigEndian = encoding === "utf-16be";
  const [win, setWin] = useState<Win | null>(null);
  const winRef = useRef<Win | null>(null);
  // Righe lette fuori dalla finestra, per inizio riga. La cache delle righe non è reattiva: `tick` ridisegna.
  const [chunks, setChunks] = useState(new Map<number, Uint8Array>());
  const [tick, setTick] = useState(0);
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
  const latest = useRef({ rows, unit, bigEndian, size, encoding, onBusy, onError });
  latest.current = { rows, unit, bigEndian, size, encoding, onBusy, onError };
  const linesRef = useRef<Line[]>([]);

  const getTop = () => useViewers.getState().states[tabId]?.top ?? 0;
  const setTop = (to: number) => {
    valid.current = { top: to, encoding: latest.current.encoding };
    useViewers.getState().set(tabId, { top: to });
    setTick((n) => n + 1); // anche senza spostamento: le righe in sospeso si richiedono di nuovo
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
  function track(total: number, done: (o: number) => number, onProgress: (o: number) => void, call: (id: number, progress: (o: number) => void) => Promise<number>) {
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

  function cancel(): boolean {
    gen.current++;
    nav.current = null;
    inflight.current.clear();
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
    cancel,
  }));

  // Cambiando codifica o smontando (cambio scheda, chiusura) si annulla tutto.
  useEffect(() => () => void cancel(), [path, encoding]);

  useEffect(() => {
    if (!ready) run(normalize());
  }, [ready, top, encoding]);

  const lines = useMemo(() => (ready && rows ? visibleLines(ctx(win), top, rows) : []), [ready, win, top, rows, tick]);
  linesRef.current = lines;

  // Finestra da rileggere, oppure la riga in fondo senza fine nota: la cerca il backend da solo.
  useEffect(() => {
    if (!lines.length) return;
    if (stale(winRef.current, top)) return void run(ensureWindow(top));
    const last = lines[lines.length - 1];
    if (last.next === null) run(nextStart(ctx(), last.start).then(() => setTick((n) => n + 1)));
  }, [lines]);

  // Le righe fuori dalla finestra si leggono insieme; si tengono solo quelle ancora visibili.
  useEffect(() => {
    const missing = lines.filter((l) => !fromWindow(win, l) && !chunks.has(l.start));
    if (!missing.length) return;
    run(
      Promise.all(missing.map((l) => readChunk(path, l.start, BYTES))).then((bufs) =>
        setChunks((prev) => {
          const keep = new Set(linesRef.current.map((l) => l.start));
          const next = new Map([...prev].filter(([s]) => keep.has(s)));
          missing.forEach((l, i) => keep.has(l.start) && next.set(l.start, bufs[i]));
          return next;
        }),
      ),
    );
  }, [lines]);

  const texts = useMemo(() => {
    const decoder = decoderFor(encoding);
    return lines.map((l) => {
      const bytes = fromWindow(win, l) ?? chunks.get(l.start);
      if (!bytes) return null;
      // Letta oltre la fine nota: si taglia al primo LF (e al CR prima).
      const t = decoder.decode(bytes);
      const i = t.indexOf("\n");
      return renderSlice(i < 0 ? t : t.slice(0, i).replace(/\r$/, ""), 0, 0, BYTES);
    });
  }, [lines, chunks, win, encoding]);

  const last = lines[lines.length - 1];
  // Dopo una riga senza fine nota le righe sono in sospeso.
  const pending = last && last.next === null ? rows - lines.length : 0;
  // Niente da scorrere in verticale: si è in cima e la riga che finisce a fine file è visibile.
  const fits = ready && top <= (bom.current?.n ?? 0) && last?.next != null && last.next >= size;

  return (
    <div className="mc-vpane">
      <div ref={viewport} className="mc-vtext" onWheel={(e) => e.deltaY && scroll(Math.sign(e.deltaY) * WHEEL_LINES)}>
        {texts.map((t, i) => (t === null ? <div key={i} className="mc-vline pending">…</div> : <div key={i} className="mc-vline">{t}</div>))}
        {Array.from({ length: Math.max(0, pending) }, (_, i) => <div key={`p${i}`} className="mc-vline pending">…</div>)}
      </div>
      {!fits && <ScrollRail fraction={size ? top / size : 0} onSeek={(f) => {
        cancel();
        run(seek(f, gen.current));
      }} />}
    </div>
  );
}
