import { ChevronRight, FolderPlus, Settings2 } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { DRIVES, folderName } from "../../lib/paths";
import { useHotlist, type HotlistItem, type MenuAnchor } from "../../state/hotlist";
import { useUi } from "../../state/ui";
import { openInActiveTab } from "../sidebar/FolderTree";
import "./dialogs.css";

type MenuRow =
  | { kind: "dir"; item: Extract<HotlistItem, { type: "dir" }> }
  | { kind: "submenu"; item: Extract<HotlistItem, { type: "submenu" }> }
  | { kind: "sep" }
  | { kind: "empty" }
  | { kind: "add" }
  | { kind: "configure" };

const selectable = (r: MenuRow) => r.kind !== "sep" && r.kind !== "empty";

function itemRows(items: HotlistItem[]): MenuRow[] {
  if (!items.length) return [{ kind: "empty" }];
  return items.map((item): MenuRow =>
    item.type === "separator" ? { kind: "sep" } : item.type === "dir" ? { kind: "dir", item } : { kind: "submenu", item },
  );
}

/** Le colonne aperte: la prima è la radice (con le azioni in fondo), le altre i sottomenu scelti in `sel`. */
function columns(items: HotlistItem[], sel: number[], canAdd: boolean): MenuRow[][] {
  const actions: MenuRow[] = [{ kind: "sep" }, ...(canAdd ? [{ kind: "add" } as MenuRow] : []), { kind: "configure" }];
  const cols = [[...itemRows(items), ...actions]];
  let cur = items;
  for (let d = 0; d < sel.length - 1; d++) {
    const row = cols[d][sel[d]];
    if (row?.kind !== "submenu") break;
    cur = row.item.items;
    cols.push(itemRows(cur));
  }
  return cols;
}

const step = (rows: MenuRow[], from: number, delta: 1 | -1): number => {
  for (let i = from + delta; i >= 0 && i < rows.length; i += delta) if (selectable(rows[i])) return i;
  return from;
};

/** Hotlist di cartelle (Ctrl+D): voci, sottomenu a cascata, "Aggiungi cartella corrente", "Configura…". */
export function HotlistMenu() {
  const anchor = useHotlist((s) => s.menu);
  return anchor ? <Menu key={`${anchor.x},${anchor.y}`} anchor={anchor} /> : null;
}

function Menu({ anchor }: { anchor: MenuAnchor }) {
  const { t } = useTranslation();
  const items = useHotlist((s) => s.items);
  const canAdd = anchor.path !== DRIVES;
  const rootRows = columns(items, [0], canAdd)[0];
  const [sel, setSel] = useState<number[]>(() => [Math.max(0, rootRows.findIndex(selectable))]);
  const [pos, setPos] = useState({ left: anchor.x, top: anchor.y });
  const ref = useRef<HTMLDivElement>(null);
  const cols = columns(items, sel, canAdd);

  const close = () => useHotlist.getState().closeMenu();
  useEffect(() => ref.current?.focus(), []);

  // Il menu resta dentro la finestra (o si centra sul punto, se non c'è una riga col cursore).
  useLayoutEffect(() => {
    const r = ref.current!.getBoundingClientRect();
    const left = anchor.center ? anchor.x - r.width / 2 : anchor.x;
    const top = anchor.center ? anchor.y - r.height / 2 : anchor.y;
    setPos({
      left: Math.max(4, Math.min(left, window.innerWidth - r.width - 4)),
      top: Math.max(4, Math.min(top, window.innerHeight - r.height - 4)),
    });
  }, [anchor, sel.length]);

  // Un click fuori dal menu lo chiude.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown, true);
    return () => document.removeEventListener("mousedown", onDown, true);
  }, []);

  // `idx` è la riga attivata nella colonna `depth`: lo stato `sel` può non averla ancora (click).
  const activate = (depth: number, idx: number, row: MenuRow) => {
    const base = [...sel.slice(0, depth), idx];
    switch (row.kind) {
      case "dir":
        close();
        openInActiveTab(row.item.path);
        break;
      case "submenu": {
        setSel([...base, columns(items, [...base, 0], canAdd)[depth + 1].findIndex(selectable)]);
        break;
      }
      case "add":
        close();
        useHotlist.getState().addDir(anchor.path, folderName(anchor.path));
        break;
      case "configure":
        close();
        useHotlist.getState().setEditing(true);
        useUi.getState().setSidebarView("favorites");
        break;
    }
  };

  const last = sel.length - 1;
  const onKeyDown = (e: KeyboardEvent) => {
    e.stopPropagation(); // i tasti restano al menu
    const rows = cols[last];
    const at = sel[last];
    const set = (i: number) => setSel([...sel.slice(0, last), i]);
    switch (e.key) {
      case "ArrowDown": set(step(rows, at, 1)); break;
      case "ArrowUp": set(step(rows, at, -1)); break;
      case "Home": set(step(rows, -1, 1)); break;
      case "End": set(step(rows, rows.length, -1)); break;
      case "ArrowRight":
        if (rows[at]?.kind === "submenu") activate(last, at, rows[at]);
        break;
      case "ArrowLeft":
        if (last > 0) setSel(sel.slice(0, last));
        break;
      case "Enter":
      case " ":
        if (rows[at]) activate(last, at, rows[at]);
        break;
      case "Escape": close(); break;
      default: return;
    }
    e.preventDefault();
  };

  const label = (row: MenuRow) => {
    if (row.kind === "add") return t("hotlist.addCurrent");
    if (row.kind === "configure") return t("hotlist.configure");
    if (row.kind === "empty") return t("hotlist.empty");
    return row.kind === "sep" ? "" : row.item.name;
  };

  return (
    <div
      ref={ref}
      className="mc-hotlist"
      style={pos}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) close();
      }}
    >
      {cols.map((rows, d) => (
        <div key={d} className="mc-hotlist-col" role="menu" aria-label={d === 0 ? t("hotlist.title") : undefined}>
          {d === 0 && <div className="mc-menu-title">{t("hotlist.title")}</div>}
          {rows.map((row, i) => {
            if (row.kind === "sep") return <div key={i} role="separator" className="mc-hotlist-sep" />;
            if (row.kind === "empty") return <div key={i} className="mc-menu-item empty">{label(row)}</div>;
            const on = i === sel[d];
            const sub = row.kind === "submenu";
            return (
              <div
                key={i}
                role="menuitem"
                aria-haspopup={sub ? "menu" : undefined}
                aria-expanded={sub ? d < last && on : undefined}
                title={row.kind === "dir" ? row.item.path : undefined}
                className={`mc-menu-item${on ? " sel" : ""}${d < last && on ? " open" : ""}`}
                onMouseDown={(e) => e.preventDefault()} // il focus resta al menu
                onMouseEnter={() => d === last && setSel([...sel.slice(0, d), i])}
                onClick={() => activate(d, i, row)}
              >
                <span className="mc-hotlist-label">
                  {row.kind === "add" && <FolderPlus size={12} aria-hidden />}
                  {row.kind === "configure" && <Settings2 size={12} aria-hidden />}
                  {label(row)}
                </span>
                {row.kind === "dir" && <span className="detail">{row.item.path}</span>}
                {sub && <ChevronRight size={12} aria-hidden />}
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
