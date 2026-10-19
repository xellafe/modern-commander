// Parti comuni ai pannelli del viewer (testo ed esadecimale).
import { useEffect, useState, type RefObject } from "react";

/** Comandi che il Viewer manda al pannello del contenuto. */
export interface PaneHandle {
  /** Di `lines` righe; negativo = verso l'inizio. */
  scroll(lines: number): void;
  page(dir: 1 | -1): void;
  /** Inizio o fine del file. */
  edge(end: boolean): void;
  /** Di `cols` colonne; negativo = verso sinistra. Solo con l'a capo spento. */
  hscroll?(cols: number): void;
  /** Di una larghezza del viewport meno una colonna. */
  hpage?(dir: 1 | -1): void;
  /** Inizio riga o fine della riga visibile più lunga. */
  hedge?(end: boolean): void;
  /** Annulla le scansioni in corso; vero se ce n'era almeno una. */
  cancel?(): boolean;
}

/** Righe per tacca della rotella. */
export const WHEEL_LINES = 3;
/** Colonne per tacca della rotella orizzontale (Maiusc+rotella o `deltaX`). */
export const WHEEL_COLS = 6;

/**
 * Righe intere che entrano in `ref`, con l'altezza dal token `--mc-row-height`. Si aggiorna al
 * ridimensionamento e al cambio di tema (che cambia l'altezza della riga); 0 finché non è misurato.
 */
export function usePaneRows(ref: RefObject<HTMLElement | null>): number {
  const [rows, setRows] = useState(0);
  useEffect(() => {
    const el = ref.current!;
    const measure = () => {
      const h = parseFloat(getComputedStyle(el).getPropertyValue("--mc-row-height")) || 22;
      setRows(Math.max(1, Math.floor(el.clientHeight / h)));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    const mo = new MutationObserver(measure);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  }, [ref]);
  return rows;
}

/**
 * Colonne intere che entrano in `ref` (padding escluso), misurate con una sonda di 10 caratteri `0` nel font del
 * pannello. Si aggiorna come `usePaneRows`; 0 finché non è misurato.
 */
export function usePaneCols(ref: RefObject<HTMLElement | null>): number {
  const [cols, setCols] = useState(0);
  useEffect(() => {
    const el = ref.current!;
    const probe = document.createElement("canvas").getContext("2d")!;
    const measure = () => {
      const s = getComputedStyle(el);
      probe.font = `${s.fontSize} ${s.fontFamily}`;
      const ch = probe.measureText("0".repeat(10)).width / 10;
      const inner = el.clientWidth - parseFloat(s.paddingLeft) - parseFloat(s.paddingRight);
      setCols(ch > 0 ? Math.max(1, Math.floor(inner / ch)) : 0);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    const mo = new MutationObserver(measure);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  }, [ref]);
  return cols;
}

/** Pixel per "riga" nei pannelli che scorrono col browser (PDF, immagine ingrandita). */
const LINE_PX = 40;

/** Comandi della tastiera per un pannello che scorre nativamente: l'elemento si legge al momento dell'uso. */
export function scrollHandle(get: () => HTMLElement | null): PaneHandle {
  return {
    scroll: (lines) => get()?.scrollBy({ top: lines * LINE_PX }),
    page: (dir) => {
      const el = get();
      el?.scrollBy({ top: dir * el.clientHeight * 0.9 });
    },
    edge: (end) => get()?.scrollTo({ top: end ? get()!.scrollHeight : 0 }),
  };
}
