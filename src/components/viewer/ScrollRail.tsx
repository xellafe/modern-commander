import { useRef, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";

/**
 * Barra di scorrimento propria: una lista virtualizzata del browser non regge file da GB (1 GB a 16 byte
 * per riga supera l'altezza massima di un elemento). `fraction` è la posizione della maniglia (0…1);
 * click e trascinamento chiamano `onSeek` con la posizione puntata. In orizzontale scorre la riga (colonna `left`).
 */
export function ScrollRail({ fraction, onSeek, orientation = "vertical" }: {
  fraction: number;
  onSeek: (fraction: number) => void;
  orientation?: "vertical" | "horizontal";
}) {
  const { t } = useTranslation();
  const rail = useRef<HTMLDivElement>(null);
  const handle = useRef<HTMLDivElement>(null);
  const f = Math.max(0, Math.min(1, fraction));
  const h = orientation === "horizontal";

  // La maniglia si centra sul puntatore: la corsa è la lunghezza della barra meno quella della maniglia.
  const seek = (e: PointerEvent) => {
    const r = rail.current!.getBoundingClientRect();
    const len = h ? handle.current!.offsetWidth : handle.current!.offsetHeight;
    const span = (h ? r.width : r.height) - len;
    const at = h ? e.clientX - r.left : e.clientY - r.top;
    onSeek(span > 0 ? Math.max(0, Math.min(1, (at - len / 2) / span)) : 0);
  };

  return (
    <div
      ref={rail}
      className={h ? "mc-rail h" : "mc-rail"}
      role="scrollbar"
      aria-orientation={orientation}
      aria-label={t(h ? "viewer.lineScrollbar" : "viewer.scrollbar")}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(f * 100)}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        seek(e);
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) seek(e);
      }}
    >
      <div ref={handle} className="mc-rail-handle" style={{ [h ? "left" : "top"]: `calc((100% - var(--mc-rail-handle)) * ${f})` }} />
    </div>
  );
}
