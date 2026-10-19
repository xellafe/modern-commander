import { WrapText } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { closeTab } from "../../commands/navigation";
import { ansiEncoding, fileInfo, readChunk } from "../../ipc/api";
import type { AppError } from "../../ipc/types";
import { ENCODINGS } from "../../lib/encoding";
import { formatSize } from "../../lib/format";
import { folderName } from "../../lib/paths";
import { useLayout } from "../../state/layout";
import { alignTop, initialViewerState, nativeMode, useViewers, type ViewerMode } from "../../state/viewers";
import { HexPane } from "./HexPane";
import type { PaneHandle } from "./pane";
import { TextPane } from "./TextPane";
import "./viewer.css";

/** Campione per scegliere modalità e codifica: non si legge altro all'apertura. */
const SAMPLE = 65536;

const toAppError = (e: unknown): AppError =>
  e && typeof e === "object" && "code" in e ? (e as AppError) : { code: "IO", path: null, detail: null };

/** F3: viewer a blocchi di una scheda di file. Lo stato (modalità, codifica, posizione) sta in `useViewers`. */
export function Viewer({ tabId, path }: { tabId: string; path: string }) {
  const { t, i18n } = useTranslation();
  const name = folderName(path);
  const state = useViewers((s) => s.states[tabId]);
  const active = useLayout((s) => s.groups.find((g) => g.tabIds.includes(tabId))?.id === s.activeGroupId);
  const [size, setSize] = useState<number | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const pane = useRef<PaneHandle>(null);

  // Apertura: dimensione e un campione; se la scheda ha già uno stato (ritorno alla scheda) non si ricalcola.
  useEffect(() => {
    let live = true;
    (async () => {
      const info = await fileInfo(path);
      if (!useViewers.getState().states[tabId]) {
        const [sample, ansi] = await Promise.all([readChunk(path, 0, SAMPLE), ansiEncoding()]);
        if (!live) return; // scheda chiusa o cambiata durante la lettura: lo stato non deve rinascere
        useViewers.getState().set(tabId, initialViewerState(name, sample, ansi, info.size <= SAMPLE));
      }
      if (live) setSize(info.size);
    })().catch((e) => live && setError(toAppError(e)));
    return () => {
      live = false;
    };
  }, [tabId, path]);

  // I tasti arrivano al contenitore che ha il focus: lo ha solo il viewer del gruppo attivo.
  useEffect(() => {
    const el = root.current!;
    if (active) el.focus({ preventScroll: true });
    else if (el.contains(document.activeElement)) (document.activeElement as HTMLElement).blur();
  }, [active]);

  const patch = (p: Partial<{ mode: ViewerMode; encoding: string; wrap: boolean }>) => {
    const cur = useViewers.getState().states[tabId]; // fresco: può arrivare da una promessa (tasto A)
    if (!cur) return;
    const next = { ...cur, ...p };
    useViewers.getState().set(tabId, { ...p, top: alignTop(cur.top, next.mode, next.encoding) });
  };
  const setEncoding = (encoding: string) => patch({ encoding, mode: "text" });

  // Come il Lister di TC. I tasti non gestiti salgono a useKeyboard (Tab, Ctrl+Tab, Ctrl+W).
  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.target !== e.currentTarget || !state || e.altKey || e.metaKey) return;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const p = pane.current;
    if (e.ctrlKey) {
      if (key !== "Home" && key !== "End") return;
      p?.edge(key === "End");
    } else if (key === "Escape") closeTab(tabId);
    else if (key === "ArrowUp") p?.scroll(-1);
    else if (key === "ArrowDown") p?.scroll(1);
    else if (key === "PageUp") p?.page(-1);
    else if (key === "PageDown") p?.page(1);
    else if (key === "Home" || key === "End") p?.edge(key === "End");
    else if (key === "1") patch({ mode: "text" });
    else if (key === "3") patch({ mode: "hex" });
    else if (key === "4") {
      const native = nativeMode(name);
      if (!native) return;
      patch({ mode: native });
    } else if (key === "7") setEncoding("utf-8");
    else if (key === "6") setEncoding("utf-16le");
    else if (key === "a") ansiEncoding().then(setEncoding).catch((err) => setError(toAppError(err)));
    else if (key === "w") patch({ wrap: !state.wrap });
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  const native = nativeMode(name);
  const modes: ViewerMode[] = ["text", "hex", ...(native ? [native] : [])];
  const encodings = state && ENCODINGS.includes(state.encoding) ? ENCODINGS : [...ENCODINGS, ...(state ? [state.encoding] : [])];
  const percent = state && size ? Math.min(100, Math.floor((state.top / size) * 100)) : 0;
  const refocus = () => root.current?.focus({ preventScroll: true });

  return (
    <div ref={root} className="mc-viewer" role="region" aria-label={name} tabIndex={0} onKeyDown={onKeyDown}>
      {state && size !== null && (
        <div className="mc-viewer-bar">
          <span className="mc-viewer-name" title={path}>{name}</span>
          <span className="mc-viewer-meta">{formatSize(size, i18n.language)}</span>
          <select
            aria-label={t("viewer.modeLabel")}
            value={state.mode}
            onChange={(e) => {
              patch({ mode: e.target.value as ViewerMode });
              refocus();
            }}
          >
            {modes.map((m) => <option key={m} value={m}>{t(`viewer.mode.${m}`)}</option>)}
          </select>
          {state.mode === "text" && (
            <>
              <select
                aria-label={t("viewer.encodingLabel")}
                value={state.encoding}
                onChange={(e) => {
                  setEncoding(e.target.value);
                  refocus();
                }}
              >
                {encodings.map((enc) => <option key={enc} value={enc}>{enc}</option>)}
              </select>
              <button
                className={state.wrap ? "on" : ""}
                aria-pressed={state.wrap}
                aria-label={t("viewer.wrap")}
                title={t("viewer.wrap")}
                onMouseDown={(e) => e.preventDefault()} // il focus resta al contenitore
                onClick={() => patch({ wrap: !state.wrap })}
              >
                <WrapText size={14} />
              </button>
            </>
          )}
          <span className="mc-viewer-pos">{t("viewer.position", { percent })}</span>
        </div>
      )}
      {error ? (
        <div className="mc-viewer-note" role="alert">
          {t(`errors.${error.code}`)}
          {error.path && <span>{error.path}</span>}
        </div>
      ) : state && size !== null ? (
        state.mode === "text" ? (
          <TextPane ref={pane} tabId={tabId} path={path} size={size} top={state.top} encoding={state.encoding} wrap={state.wrap} onError={(e) => setError(toAppError(e))} />
        ) : state.mode === "hex" ? (
          <HexPane ref={pane} tabId={tabId} path={path} size={size} top={state.top} onError={(e) => setError(toAppError(e))} />
        ) : (
          <div className="mc-viewer-note">{t("viewer.nativePending")}</div>
        )
      ) : null}
    </div>
  );
}
