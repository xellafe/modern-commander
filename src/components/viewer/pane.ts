// Parti comuni ai pannelli del viewer (testo ed esadecimale).
import { useEffect, useState, type RefObject } from "react";

/** Comandi che il Viewer manda al pannello del contenuto. */
export interface PaneHandle {
  /** Di `lines` righe; negativo = verso l'inizio. */
  scroll(lines: number): void;
  page(dir: 1 | -1): void;
  /** Inizio o fine del file. */
  edge(end: boolean): void;
}

/** Righe per tacca della rotella. */
export const WHEEL_LINES = 3;

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
