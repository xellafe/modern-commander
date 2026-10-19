import { useVirtualizer } from "@tanstack/react-virtual";
import type { TFunction } from "i18next";
import { ArrowDown, ArrowUp, TriangleAlert } from "lucide-react";
import { memo, useEffect, useRef, useState, useSyncExternalStore, type MouseEvent, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { nameError, renameInPlace } from "../../commands/fileOps";
import { setGridColumns, setPageSize, sortBy } from "../../commands/navigation";
import { executeCommand } from "../../commands/registry";
import { rowContextMenu, trackDragOut } from "../../commands/shell";
import { formatAttrs, formatDate, formatSize, splitName } from "../../lib/format";
import { joinPath } from "../../lib/paths";
import { useLayout } from "../../state/layout";
import { hasSize, markRange, toggleMark, type Row, type SortKey, type TabView } from "../../state/tabView";
import { useTabViews } from "../../state/tabViews";
import { THUMB_SIZE, thumbKey, thumbs } from "../../state/thumbnails";
import { FileIcon } from "./FileIcon";

const BRIEF_COL_WIDTH = 220;
const THUMB_TILE_W = 112;
const THUMB_TILE_H = 136; // 4 + immagine (96) + 4 + due righe di nome (28) + 4

// Altezza della riga dal token --mc-row-height: cambia col tema, si rilegge quando cambia data-theme.
let rowHeightCache: number | null = null;
const subscribeTheme = (cb: () => void) => {
  const mo = new MutationObserver(() => {
    rowHeightCache = null;
    cb();
  });
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => mo.disconnect();
};
const readRowHeight = () =>
  (rowHeightCache ??= parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--mc-row-height")) || 22);

function useSize(ref: RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const ro = new ResizeObserver(([e]) => {
      const { width, height } = e.contentRect;
      setSize((s) => (s.width === width && s.height === height ? s : { width, height }));
    });
    if (ref.current) ro.observe(ref.current);
    return () => ro.disconnect();
  }, [ref]);
  return size;
}
const useHeight = (ref: RefObject<HTMLElement | null>) => useSize(ref).height;

// Mouse come in TC: click = cursore, Ctrl+click = marca, Shift+click = intervallo.
function clickRow(tabId: string, i: number, e: MouseEvent) {
  if (e.button !== 0) return;
  useTabViews.getState().update(tabId, (v) => {
    if (e.ctrlKey) return { ...toggleMark(v, v.rows[i].name), cursor: i };
    if (e.shiftKey) return { ...markRange(v, v.cursor, i), cursor: i };
    return v.cursor === i ? v : { ...v, cursor: i };
  });
}
const openRow = () => void executeCommand("cm_Return");

// Click, doppio click, tasto destro (menu della shell) e drag in uscita verso Explorer.
const rowHandlers = (tabId: string, i: number) => ({
  onMouseDown: (e: MouseEvent) => {
    clickRow(tabId, i, e);
    trackDragOut(tabId, i, e);
  },
  onDoubleClick: openRow,
  onContextMenu: (e: MouseEvent) => {
    e.stopPropagation();
    rowContextMenu(tabId, i, e);
  },
});

const rowClass = (v: TabView, i: number, row: Row) =>
  `mc-row${i === v.cursor ? " cur" : ""}${v.marked.has(row.name) ? " mk" : ""}`;

interface ListProps {
  tabId: string;
  view: TabView;
  active: boolean;
  rowHeight: number;
}

// Shift+F6: Invio rinomina, Esc (o un click altrove) annulla; è selezionato il nome senza estensione.
function RenameInput({ tabId, row }: { tabId: string; row: Row }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState(row.name);
  const error = nameError(value.trim());
  useEffect(() => {
    ref.current!.focus();
    ref.current!.setSelectionRange(0, splitName(row.name, row.isDir)[0].length);
  }, []);
  const cancel = () => useTabViews.getState().setOverlay(null);
  return (
    <span className="mc-rename">
      <input
        ref={ref}
        value={value}
        spellCheck={false}
        aria-label={t("ops.renameLabel")}
        aria-invalid={!!error}
        onChange={(e) => setValue(e.target.value)}
        onBlur={cancel}
        onMouseDown={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onContextMenu={(e) => e.stopPropagation()} // menu di modifica del testo di WebView2
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") cancel();
          else if (e.key === "Enter" && !error) {
            const base = useLayout.getState().tabs[tabId].path;
            renameInPlace(tabId, base, row.name, value.trim()).catch(console.error);
          } else return;
          e.preventDefault();
        }}
      />
      {error && <span className="mc-rename-error" role="alert">{t(error)}</span>}
    </span>
  );
}

const useRenaming = (tabId: string) =>
  useTabViews((s) => (s.overlay?.kind === "rename" && s.overlay.tabId === tabId ? s.overlay.name : null));

export function FileList({ tabId, active }: { tabId: string; active: boolean }) {
  const view = useTabViews((s) => s.views[tabId]);
  const rowHeight = useSyncExternalStore(subscribeTheme, readRowHeight);
  if (!view) return <div className="mc-list" />;
  const props = { tabId, view, active, rowHeight };
  return view.view === "thumbs" ? <ThumbList {...props} /> : view.view === "brief" ? <BriefList {...props} /> : <FullList {...props} />;
}

function sizeText(row: Row, dirSize: number | undefined, sizing: boolean, lng: string, t: TFunction): string {
  if (row.isParent) return "";
  if (!row.isDir) return formatSize(row.size, lng);
  if (sizing) return "…";
  return dirSize === undefined ? t("groups.dir") : formatSize(dirSize, lng);
}

function FullList({ tabId, view, active, rowHeight }: ListProps) {
  const { t, i18n } = useTranslation();
  const lng = i18n.language;
  const ref = useRef<HTMLDivElement>(null);
  const height = useHeight(ref);
  const renaming = useRenaming(tabId);
  const virt = useVirtualizer({
    count: view.rows.length,
    getScrollElement: () => ref.current,
    estimateSize: () => rowHeight,
    overscan: 12,
  });
  useEffect(() => virt.measure(), [virt, rowHeight]);
  const perPage = Math.max(1, Math.floor(height / rowHeight));
  useEffect(() => {
    if (active) setPageSize(perPage);
  }, [active, perPage]);
  useEffect(() => {
    if (view.rows.length) virt.scrollToIndex(view.cursor, { align: "auto" });
  }, [virt, view.cursor, view.rows]);

  const head = (key: SortKey | null, label: string, cls: string) => (
    <span className={cls} onClick={key ? () => sortBy(tabId, key) : undefined}>
      {label}
      {key && view.sort.key === key && (view.sort.desc ? <ArrowDown size={10} /> : <ArrowUp size={10} />)}
    </span>
  );

  return (
    <div className="mc-list mc-list-full">
      <div className="mc-list-head mc-cols">
        <span />
        {head("name", t("groups.columns.name"), "c-name")}
        {head("ext", t("groups.columns.ext"), "c-ext")}
        {head("size", t("groups.columns.size"), "c-size")}
        {head("date", t("groups.columns.date"), "c-date")}
        {head(null, t("groups.columns.attributes"), "c-attr")}
      </div>
      <div className="mc-list-scroll" ref={ref} onContextMenu={(e) => rowContextMenu(tabId, null, e)}>
        <div className="mc-list-canvas" style={{ height: virt.getTotalSize() }}>
          {virt.getVirtualItems().map(({ index: i, start }) => {
            const row = view.rows[i];
            return (
              <FullRow
                key={i}
                tabId={tabId}
                row={row}
                i={i}
                start={start}
                height={rowHeight}
                cls={rowClass(view, i, row)}
                dirSize={hasSize(view.dirSizes, row.name) ? view.dirSizes[row.name] : undefined}
                sizing={view.sizing.has(row.name)}
                skipped={hasSize(view.skipped, row.name) ? view.skipped[row.name] : 0}
                lng={lng}
                renaming={renaming === row.name}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

interface RowProps {
  tabId: string;
  row: Row;
  i: number;
  start: number;
  height: number;
  cls: string;
  dirSize: number | undefined;
  sizing: boolean;
  skipped: number;
  lng: string;
  renaming: boolean;
}

// memo: spostando il cursore si ridisegnano solo le due righe che cambiano classe.
const FullRow = memo(function FullRow({ tabId, row, i, start, height, cls, dirSize, sizing, skipped, lng, renaming }: RowProps) {
  const { t } = useTranslation();  const [base, ext] = row.isParent || row.drive ? [row.name, ""] : splitName(row.name, row.isDir);
  const d = row.drive;
  return (
    <div
      className={`${cls} mc-cols${renaming ? " renaming" : ""}`}
      style={{ transform: `translateY(${start}px)`, height }}
      {...rowHandlers(tabId, i)}
    >
      <span className="c-icon"><FileIcon row={row} /></span>
      <span className="c-name">{renaming ? <RenameInput tabId={tabId} row={row} /> : base}</span>
      {d ? (
        <span className="c-drive">
          {[d.label, d.ready && t("groups.driveSpace", { free: formatSize(d.free, lng), total: formatSize(d.total, lng) })]
            .filter(Boolean).join(" · ")}
        </span>
      ) : (
        <>
          <span className="c-ext">{ext}</span>
          <span className={`c-size${sizing ? " sizing" : ""}`} title={sizing ? t("groups.sizing") : undefined}>
            {skipped > 0 && (
              <span className="mc-skipped" title={t("groups.skipped", { count: skipped })} role="img" aria-label={t("groups.skipped", { count: skipped })}>
                <TriangleAlert size={10} aria-hidden />
              </span>
            )}
            {sizeText(row, dirSize, sizing, lng, t)}
          </span>
          <span className="c-date">{row.isParent ? "" : formatDate(row.modifiedMs, lng)}</span>
          <span className="c-attr">{row.isParent ? "" : formatAttrs(row.attributes)}</span>
        </>
      )}
    </div>
  );
});

// Vista breve (Ctrl+F1): solo icona e nome, in colonne che scorrono in orizzontale come in TC.
function BriefList({ tabId, view, active, rowHeight }: ListProps) {
  const ref = useRef<HTMLDivElement>(null);
  const height = useHeight(ref);
  const renaming = useRenaming(tabId);
  const perCol = Math.max(1, Math.floor(height / rowHeight));
  const virt = useVirtualizer({
    horizontal: true,
    count: Math.ceil(view.rows.length / perCol),
    getScrollElement: () => ref.current,
    estimateSize: () => BRIEF_COL_WIDTH,
    overscan: 2,
  });
  useEffect(() => {
    if (active) setPageSize(perCol);
  }, [active, perCol]);
  useEffect(() => {
    if (view.rows.length) virt.scrollToIndex(Math.floor(view.cursor / perCol), { align: "auto" });
  }, [virt, view.cursor, view.rows, perCol]);

  return (
    <div className="mc-list mc-list-brief">
      <div
        className="mc-list-scroll"
        ref={ref}
        onWheel={(e) => (e.currentTarget.scrollLeft += e.deltaY)}
        onContextMenu={(e) => rowContextMenu(tabId, null, e)}
      >
        <div className="mc-list-canvas" style={{ width: virt.getTotalSize() }}>
          {virt.getVirtualItems().map(({ index: c, start }) => (
            <div key={c} className="mc-brief-col" style={{ transform: `translateX(${start}px)`, width: BRIEF_COL_WIDTH }}>
              {view.rows.slice(c * perCol, (c + 1) * perCol).map((row, k) => {
                const i = c * perCol + k;
                return (
                  <div
                    key={i}
                    className={rowClass(view, i, row)}
                    style={{ height: rowHeight }}
                    {...rowHandlers(tabId, i)}
                  >
                    <span className="c-icon"><FileIcon row={row} /></span>
                    <span className="c-name">{renaming === row.name ? <RenameInput tabId={tabId} row={row} /> : row.name}</span>
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// Vista miniature (Ctrl+Maiusc+F1): griglia di riquadri virtualizzata per righe di riquadri. Il cursore si muove
// sempre sui dati (`cursorStep`), le miniature arrivano dopo, dalla coda: nessun tasto aspetta un'anteprima.
function ThumbList({ tabId, view, active }: ListProps) {
  const ref = useRef<HTMLDivElement>(null);
  const { width, height } = useSize(ref);
  const renaming = useRenaming(tabId);
  const dir = view.history[view.historyIndex];
  const cols = Math.max(1, Math.floor(width / THUMB_TILE_W));
  const perPage = Math.max(1, Math.floor(height / THUMB_TILE_H));
  const virt = useVirtualizer({
    count: Math.ceil(view.rows.length / cols),
    getScrollElement: () => ref.current,
    estimateSize: () => THUMB_TILE_H,
    overscan: 2,
  });
  useEffect(() => {
    if (active) {
      setPageSize(perPage);
      setGridColumns(cols);
    }
  }, [active, perPage, cols]);
  useEffect(() => {
    if (view.rows.length) virt.scrollToIndex(Math.floor(view.cursor / cols), { align: "auto" });
  }, [virt, view.cursor, view.rows, cols]);
  // Cambio di cartella o di vista (smontaggio): le richieste ancora in coda non servono più.
  useEffect(() => () => thumbs.cancelOwner(tabId), [tabId, dir]);

  return (
    <div className="mc-list mc-list-thumbs">
      <div className="mc-list-scroll" ref={ref} onContextMenu={(e) => rowContextMenu(tabId, null, e)}>
        <div className="mc-list-canvas" style={{ height: virt.getTotalSize() }}>
          {virt.getVirtualItems().map(({ index: r, start }) => (
            <div key={r} className="mc-thumb-row" style={{ transform: `translateY(${start}px)`, height: THUMB_TILE_H }}>
              {view.rows.slice(r * cols, (r + 1) * cols).map((row, k) => {
                const i = r * cols + k;
                return <Tile key={i} tabId={tabId} row={row} i={i} dir={dir} cls={rowClass(view, i, row)} renaming={renaming === row.name} />;
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

interface TileProps {
  tabId: string;
  row: Row;
  i: number;
  dir: string;
  cls: string;
  renaming: boolean;
}

// memo: spostando il cursore si ridisegnano solo i due riquadri che cambiano classe.
const Tile = memo(function Tile({ tabId, row, i, dir, cls, renaming }: TileProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [ready, setReady] = useState(false);
  const hasThumb = !row.isDir && !row.isParent && !row.drive;
  const path = hasThumb ? joinPath(dir, row.name) : "";
  useEffect(() => {
    setReady(false);
    if (!hasThumb) return;
    let live = true;
    // Alla risoluzione fisica: con la scala di Windows al 125% o 150% i 96 px logici sono più pixel.
    const size = Math.min(1024, Math.ceil(THUMB_SIZE * (window.devicePixelRatio || 1)));
    const key = thumbKey(path, row.modifiedMs, size);
    void thumbs.request(tabId, key, path, size).then((bmp) => {
      const c = canvas.current;
      if (!live || !bmp || !c) return;
      c.width = bmp.width;
      c.height = bmp.height;
      c.getContext("2d")!.drawImage(bmp, 0, 0);
      setReady(true);
    });
    // Il riquadro esce dalla vista: se la richiesta non è partita lascia il posto a quelle dei riquadri visibili.
    return () => {
      live = false;
      thumbs.release(tabId, key);
    };
  }, [tabId, hasThumb, path, row.modifiedMs]);

  return (
    <div
      className={`${cls} mc-thumb${renaming ? " renaming" : ""}`}
      style={{ width: THUMB_TILE_W, height: THUMB_TILE_H }}
      {...rowHandlers(tabId, i)}
    >
      <span className="mc-thumb-img" style={{ width: THUMB_SIZE, height: THUMB_SIZE }}>
        {!ready && <FileIcon row={row} size={hasThumb ? 32 : 48} />}
        <canvas ref={canvas} style={ready ? undefined : { display: "none" }} />
      </span>
      <span className="c-name" title={row.name}>{renaming ? <RenameInput tabId={tabId} row={row} /> : row.name}</span>
    </div>
  );
});
