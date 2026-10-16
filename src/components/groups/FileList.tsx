import { useVirtualizer } from "@tanstack/react-virtual";
import type { TFunction } from "i18next";
import { ArrowDown, ArrowUp } from "lucide-react";
import { memo, useEffect, useRef, useState, useSyncExternalStore, type MouseEvent, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { setPageSize, sortBy } from "../../commands/navigation";
import { executeCommand } from "../../commands/registry";
import { formatAttrs, formatDate, formatSize, splitName } from "../../lib/format";
import { markRange, toggleMark, type Row, type SortKey, type TabView } from "../../state/tabView";
import { useTabViews } from "../../state/tabViews";
import { FileIcon } from "./FileIcon";

const BRIEF_COL_WIDTH = 220;

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

function useHeight(ref: RefObject<HTMLElement | null>) {
  const [h, setH] = useState(0);
  useEffect(() => {
    const ro = new ResizeObserver(([e]) => setH(e.contentRect.height));
    if (ref.current) ro.observe(ref.current);
    return () => ro.disconnect();
  }, [ref]);
  return h;
}

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

const rowClass = (v: TabView, i: number, row: Row) =>
  `mc-row${i === v.cursor ? " cur" : ""}${v.marked.has(row.name) ? " mk" : ""}`;

interface ListProps {
  tabId: string;
  view: TabView;
  active: boolean;
  rowHeight: number;
}

export function FileList({ tabId, active }: { tabId: string; active: boolean }) {
  const view = useTabViews((s) => s.views[tabId]);
  const rowHeight = useSyncExternalStore(subscribeTheme, readRowHeight);
  if (!view) return <div className="mc-list" />;
  const props = { tabId, view, active, rowHeight };
  return view.view === "brief" ? <BriefList {...props} /> : <FullList {...props} />;
}

function sizeText(row: Row, dirSize: number | undefined, lng: string, t: TFunction): string {
  if (row.isParent) return "";
  if (!row.isDir) return formatSize(row.size, lng);
  return dirSize === undefined ? t("groups.dir") : formatSize(dirSize, lng);
}

function FullList({ tabId, view, active, rowHeight }: ListProps) {
  const { t, i18n } = useTranslation();
  const lng = i18n.language;
  const ref = useRef<HTMLDivElement>(null);
  const height = useHeight(ref);
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
      <div className="mc-list-scroll" ref={ref}>
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
                dirSize={view.dirSizes[row.name]}
                lng={lng}
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
  lng: string;
}

// memo: spostando il cursore si ridisegnano solo le due righe che cambiano classe.
const FullRow = memo(function FullRow({ tabId, row, i, start, height, cls, dirSize, lng }: RowProps) {
  const { t } = useTranslation();  const [base, ext] = row.isParent || row.drive ? [row.name, ""] : splitName(row.name, row.isDir);
  const d = row.drive;
  return (
    <div
      className={`${cls} mc-cols`}
      style={{ transform: `translateY(${start}px)`, height }}
      onMouseDown={(e) => clickRow(tabId, i, e)}
      onDoubleClick={openRow}
    >
      <span className="c-icon"><FileIcon row={row} /></span>
      <span className="c-name">{base}</span>
      {d ? (
        <span className="c-drive">
          {[d.label, d.ready && t("groups.driveSpace", { free: formatSize(d.free, lng), total: formatSize(d.total, lng) })]
            .filter(Boolean).join(" · ")}
        </span>
      ) : (
        <>
          <span className="c-ext">{ext}</span>
          <span className="c-size">{sizeText(row, dirSize, lng, t)}</span>
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
      <div className="mc-list-scroll" ref={ref} onWheel={(e) => (e.currentTarget.scrollLeft += e.deltaY)}>
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
                    onMouseDown={(e) => clickRow(tabId, i, e)}
                    onDoubleClick={openRow}
                  >
                    <span className="c-icon"><FileIcon row={row} /></span>
                    <span className="c-name">{row.name}</span>
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
