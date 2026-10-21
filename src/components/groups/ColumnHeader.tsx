import { ArrowDown, ArrowUp, Check } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { sortBy } from "../../commands/navigation";
import {
  COLUMN_DEFS,
  DEFAULT_COLUMNS,
  gridTemplate,
  moveColumn,
  resizeColumn,
  toggleColumn,
  visibleColumns,
  type ColumnConfig,
  type ColumnId,
} from "../../lib/columns";
import { useSettings } from "../../state/settings";
import type { TabView } from "../../state/tabView";

export const cellClass = (id: ColumnId) => `c-${id}${COLUMN_DEFS[id].align === "right" ? " c-right" : ""}`;

const DROP_CLASSES = ["drop-before", "drop-after"];
const settings = () => useSettings.getState();

/** Colonna visibile sotto il puntatore e lato di caduta; `beforeId` è la colonna davanti a cui va inserita (null = in fondo). */
function dropTarget(head: HTMLElement, x: number, ids: ColumnId[]) {
  for (const el of head.querySelectorAll<HTMLElement>("[data-col]")) {
    const r = el.getBoundingClientRect();
    if (x < r.left || x > r.right) continue;
    const id = el.dataset.col as ColumnId;
    const after = x >= r.left + r.width / 2;
    return { el, cls: after ? "drop-after" : "drop-before", beforeId: after ? (ids[ids.indexOf(id) + 1] ?? null) : id };
  }
  return null;
}

interface Props {
  tabId: string;
  sort: TabView["sort"];
  visible: ColumnConfig[];
  template: string;
  /** Griglia provvisoria durante il ridimensionamento (null al rilascio): righe e intestazione si allargano insieme. */
  onLiveTemplate: (template: string | null) => void;
}

// Intestazione della vista dettagliata: click = ordina, trascinamento = sposta (pointer events come le schede,
// il DnD HTML5 su Windows richiede di spegnere il drop da Explorer), maniglia a destra = larghezza, tasto destro = menu.
export function ColumnHeader({ tabId, sort, visible, template, onLiveTemplate }: Props) {
  const { t } = useTranslation();
  const head = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const ids = visible.map((c) => c.id);

  const startDrag = (e: PointerEvent<HTMLElement>, id: ColumnId) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const x0 = e.clientX;
    let dragging = false;
    const clear = () => head.current?.querySelectorAll(".drop-before, .drop-after").forEach((n) => n.classList.remove(...DROP_CLASSES));
    const move = (ev: globalThis.PointerEvent) => {
      if (!dragging && Math.abs(ev.clientX - x0) < 5) return;
      dragging = true;
      clear();
      const tg = head.current && dropTarget(head.current, ev.clientX, ids);
      if (tg && tg.el.dataset.col !== id) tg.el.classList.add(tg.cls);
    };
    const stop = (ev: globalThis.PointerEvent) => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", stop);
      el.removeEventListener("pointercancel", stop);
      clear();
      if (ev.type !== "pointerup") return;
      if (!dragging) {
        const key = COLUMN_DEFS[id].sortKey;
        if (key) sortBy(tabId, key);
        return;
      }
      const tg = head.current && dropTarget(head.current, ev.clientX, ids);
      if (tg && tg.el.dataset.col !== id) void settings().update({ columns: moveColumn(settings().columns, id, tg.beforeId) });
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", stop);
    el.addEventListener("pointercancel", stop);
  };

  const startResize = (e: PointerEvent<HTMLElement>, id: ColumnId, width: number) => {
    if (e.button !== 0) return;
    e.stopPropagation(); // né spostamento né ordinamento
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const x0 = e.clientX;
    const resized = (x: number) => resizeColumn(settings().columns, id, width + x - x0);
    const move = (ev: globalThis.PointerEvent) =>
      onLiveTemplate(gridTemplate(visibleColumns(resized(ev.clientX), settings().extInName)));
    const stop = (ev: globalThis.PointerEvent) => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", stop);
      el.removeEventListener("pointercancel", stop);
      void settings().update({ columns: resized(ev.clientX) }); // una sola scrittura, al rilascio
      onLiveTemplate(null);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", stop);
    el.addEventListener("pointercancel", stop);
  };

  return (
    <>
      <div
        ref={head}
        className="mc-list-head mc-cols"
        style={{ gridTemplateColumns: template }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
      >
        <span />
        {visible.map(({ id, width }) => {
          const key = COLUMN_DEFS[id].sortKey;
          return (
            <span key={id} data-col={id} className={cellClass(id)} onPointerDown={(e) => startDrag(e, id)}>
              <span className="mc-col-label">{t(`groups.columns.${id}`)}</span>
              {key && sort.key === key && (sort.desc ? <ArrowDown size={10} /> : <ArrowUp size={10} />)}
              {width !== null && <i className="mc-col-grip" onPointerDown={(e) => startResize(e, id, width)} />}
            </span>
          );
        })}
      </div>
      {menu && <ColumnMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)} />}
    </>
  );
}

// Menu delle colonne: resta aperto dopo ogni scelta, si chiude con Esc o con un click fuori (blur).
function ColumnMenu({ x, y, onClose }: { x: number; y: number; onClose: () => void }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const columns = useSettings((s) => s.columns);
  const extInName = useSettings((s) => s.extInName);
  useEffect(() => ref.current?.focus(), []);

  const onKeyDown = (e: KeyboardEvent) => {
    e.stopPropagation(); // i tasti restano al menu
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };
  // checked null = voce semplice (Ripristina), non una spunta
  const item = (key: string, label: string, checked: boolean | null, disabled: boolean, act: () => void, hint?: string) => (
    <div
      key={key}
      role={checked === null ? "menuitem" : "menuitemcheckbox"}
      aria-checked={checked ?? undefined}
      aria-disabled={disabled}
      className={`mc-menu-item mc-col-item${disabled ? " disabled" : ""}`}
      onMouseDown={(e) => e.preventDefault()}
      onClick={disabled ? undefined : act}
    >
      <span className="mc-col-check">{checked && <Check size={12} aria-hidden />}</span>
      <span>{label}</span>
      {hint && <span className="detail">{hint}</span>}
    </div>
  );
  const update = settings().update;

  return (
    <div
      ref={ref}
      className="mc-menu mc-col-menu"
      style={{ left: Math.min(x, window.innerWidth - 300), top: y }}
      role="dialog"
      aria-label={t("groups.columnsMenu.title")}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onBlur={onClose}
    >
      <div className="mc-menu-title">{t("groups.columnsMenu.title")}</div>
      <div role="menu">
        {columns.map((c) => {
          const extHidden = c.id === "ext" && extInName;
          return item(
            c.id,
            t(`groups.columnsMenu.columns.${c.id}`),
            c.visible && !extHidden,
            c.id === "name" || extHidden,
            () => void update({ columns: toggleColumn(settings().columns, c.id) }),
            c.id === "name" ? t("groups.columnsMenu.alwaysVisible") : undefined,
          );
        })}
        <div className="mc-menu-sep" role="separator" />
        {item("extInName", t("groups.columnsMenu.extInName"), extInName, false, () => void update({ extInName: !settings().extInName }))}
        <div className="mc-menu-sep" role="separator" />
        {item("reset", t("groups.columnsMenu.reset"), null, false, () => void update({ columns: DEFAULT_COLUMNS, extInName: false }))}
      </div>
    </div>
  );
}
