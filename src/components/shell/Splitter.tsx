import type { PointerEvent } from "react";

interface Props {
  axis: "x" | "y";
  /** Dimensione corrente dell'elemento ridimensionato. */
  size: number;
  onResize: (size: number) => void;
  /** Il bordo trascinato è quello opposto all'origine (es. pannello in basso: salendo cresce). */
  invert?: boolean;
}

// Splitter a trascinamento con pointer capture: segue il puntatore anche fuori dalla maniglia.
export function Splitter({ axis, size, onResize, invert }: Props) {
  const pos = (e: PointerEvent) => (axis === "x" ? e.clientX : e.clientY);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    const startPos = pos(e);
    const startSize = size;
    const move = (ev: globalThis.PointerEvent) => {
      const delta = (axis === "x" ? ev.clientX : ev.clientY) - startPos;
      onResize(startSize + (invert ? -delta : delta));
    };
    const el = e.currentTarget;
    const stop = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", stop);
      el.removeEventListener("pointercancel", stop);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", stop);
    el.addEventListener("pointercancel", stop);
  };

  return <div className={`mc-splitter mc-splitter-${axis}`} onPointerDown={onPointerDown} />;
}
