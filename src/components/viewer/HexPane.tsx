import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { readChunk } from "../../ipc/api";
import { hexRow, offsetDigits } from "../../lib/hex";
import { useViewers } from "../../state/viewers";
import { usePaneRows, WHEEL_LINES, type PaneHandle } from "./pane";
import { ScrollRail } from "./ScrollRail";

const ROW_BYTES = 16;

interface Props {
  tabId: string;
  path: string;
  size: number;
  top: number;
  onError: (e: unknown) => void;
  ref?: Ref<PaneHandle>;
}

/** Vista esadecimale: 16 byte per riga (offset, byte, ASCII). Legge solo le righe visibili. */
export function HexPane({ tabId, path, size, top, onError, ref }: Props) {
  const viewport = useRef<HTMLDivElement>(null);
  const rows = usePaneRows(viewport);
  const [data, setData] = useState<{ top: number; bytes: Uint8Array } | null>(null);
  const seq = useRef(0);
  const total = Math.ceil(size / ROW_BYTES);
  const maxTop = Math.max(0, (total - rows) * ROW_BYTES);

  // Una lettura per ogni posizione; una risposta superata da una richiesta più recente si scarta.
  useEffect(() => {
    if (!rows) return;
    const mine = ++seq.current;
    readChunk(path, top, (rows + 1) * ROW_BYTES)
      .then((bytes) => mine === seq.current && setData({ top, bytes }))
      .catch((e) => mine === seq.current && onError(e));
  }, [path, top, rows]);

  // `top` resta multiplo di 16 e dentro il file.
  const go = (to: number) => {
    const aligned = Math.max(0, Math.min(to, maxTop));
    useViewers.getState().set(tabId, { top: aligned - (aligned % ROW_BYTES) });
  };
  const current = () => useViewers.getState().states[tabId]?.top ?? 0;
  const scroll = (lines: number) => go(current() + lines * ROW_BYTES);

  useImperativeHandle(ref, () => ({
    scroll,
    page: (dir) => scroll(dir * Math.max(1, rows - 1)),
    edge: (end) => go(end ? maxTop : 0),
  }));

  const digits = offsetDigits(size);
  const lines = data ? Array.from({ length: Math.ceil(data.bytes.length / ROW_BYTES) }, (_, i) =>
    hexRow(data.bytes.subarray(i * ROW_BYTES, (i + 1) * ROW_BYTES), data.top + i * ROW_BYTES, digits)) : [];

  return (
    <div className="mc-vpane">
      <div ref={viewport} className="mc-vhex" onWheel={(e) => e.deltaY && scroll(Math.sign(e.deltaY) * WHEEL_LINES)}>
        {lines.map((l) => (
          <div key={l.offset} className="mc-vline">
            <span className="off">{l.offset}</span>
            <span className="hex">{l.hex}</span>
            <span className="asc">{l.ascii}</span>
          </div>
        ))}
      </div>
      <ScrollRail
        fraction={size ? top / size : 0}
        onSeek={(f) => go(Math.floor(f * total) * ROW_BYTES)}
      />
    </div>
  );
}
