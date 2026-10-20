import { Check, ChevronDown, ChevronRight, Folder, FolderPlus, FolderTree, Pencil, Star, Trash2 } from "lucide-react";
import { useRef, useState, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { DRIVES, folderName } from "../../lib/paths";
import { resolveDrop, useHotlist, type HotlistItem } from "../../state/hotlist";
import { activeTab, useLayout } from "../../state/layout";
import { openInActiveTab } from "./FolderTree";

type Row = { item: HotlistItem; depth: number };

/** Righe visibili: i sottomenu chiusi nascondono i figli (in modifica sono sempre aperti). */
function flatten(items: HotlistItem[], open: (id: string) => boolean, depth = 0): Row[] {
  return items.flatMap((item) => [
    { item, depth },
    ...(item.type === "submenu" && open(item.id) ? flatten(item.items, open, depth + 1) : []),
  ]);
}

// Trascinamento con pointer events (come le schede): il DnD HTML5 non è usabile in questa app.
const MARKS = ["drop-before", "drop-after", "drop-into", "drop-end"];
const clearMarks = () => document.querySelectorAll(".mc-fav, .mc-fav [data-hl-id]").forEach((n) => n.classList.remove(...MARKS));

type Drop = { toParentId: string | null; index: number };

function dropAt(x: number, y: number, dragId: string): { drop: Drop; el: Element; cls: string } | null {
  const hit = document.elementFromPoint(x, y);
  const rowEl = hit?.closest<HTMLElement>("[data-hl-id]");
  const items = useHotlist.getState().items;
  if (rowEl) {
    const r = rowEl.getBoundingClientRect();
    const f = (y - r.top) / r.height;
    const isSub = rowEl.dataset.hlSub === "1";
    const where = isSub ? (f < 0.25 ? "before" : f > 0.75 ? "after" : "into") : f < 0.5 ? "before" : "after";
    const drop = resolveDrop(items, dragId, rowEl.dataset.hlId!, where);
    return drop && { drop, el: rowEl, cls: `drop-${where}` };
  }
  // Spazio vuoto sotto l'ultima riga: in fondo alla radice.
  const root = hit?.closest<HTMLElement>(".mc-fav");
  return root ? { drop: { toParentId: null, index: items.filter((i) => i.id !== dragId).length }, el: root, cls: "drop-end" } : null;
}

function startDrag(e: PointerEvent<HTMLElement>, id: string) {
  if (e.button !== 0 || (e.target as Element).closest("button, input")) return;
  const el = e.currentTarget;
  el.setPointerCapture(e.pointerId);
  const x0 = e.clientX;
  const y0 = e.clientY;
  let dragging = false;
  const move = (ev: globalThis.PointerEvent) => {
    if (!dragging && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 5) return;
    dragging = true;
    el.classList.add("dragging");
    clearMarks();
    const t = dropAt(ev.clientX, ev.clientY, id);
    t?.el.classList.add(t.cls);
  };
  const stop = (ev: globalThis.PointerEvent) => {
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerup", stop);
    el.removeEventListener("pointercancel", stop);
    el.classList.remove("dragging");
    clearMarks();
    const t = dragging && ev.type === "pointerup" ? dropAt(ev.clientX, ev.clientY, id) : null;
    if (t) useHotlist.getState().move(id, t.drop.toParentId, t.drop.index);
  };
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", stop);
  el.addEventListener("pointercancel", stop);
}

// Invio conferma, Esc annulla, la perdita del focus conferma; un nome vuoto non cambia niente.
function RenameInput({ initial, onDone }: { initial: string; onDone: (name: string | null) => void }) {
  const { t } = useTranslation();
  const cancelled = useRef(false);
  return (
    <input
      className="mc-tree-rename"
      aria-label={t("hotlist.rename")}
      defaultValue={initial}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => onDone(cancelled.current ? null : e.currentTarget.value.trim() || null)}
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== "Escape") return;
        e.stopPropagation();
        cancelled.current = e.key === "Escape";
        e.currentTarget.blur();
      }}
    />
  );
}

interface ListProps {
  edit: boolean;
  /** Occupa tutta l'altezza della vista: lo spazio vuoto in fondo è una zona di rilascio (radice). */
  fill?: boolean;
  /** Voce in rinomina (stato nel genitore, così "Nuovo sottomenu" può avviarla). */
  renaming?: string | null;
  setRenaming?: (id: string | null) => void;
}

export function FavoritesList({ edit, fill, renaming = null, setRenaming = () => {} }: ListProps) {
  const { t } = useTranslation();
  const items = useHotlist((s) => s.items);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const rows = flatten(items, (id) => edit || open.has(id));
  const toggle = (id: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (!n.delete(id)) n.add(id);
      return n;
    });

  if (!rows.length) return <div className="mc-side-empty">{t("hotlist.empty")}</div>;
  return (
    <div className={`mc-fav${fill ? " fill" : ""}`} role="tree">
      {rows.map(({ item, depth }) => {
        if (item.type === "separator") {
          return (
            <div key={item.id} data-hl-id={item.id} role="separator" className="mc-tree-sep" onPointerDown={edit ? (e) => startDrag(e, item.id) : undefined}>
              {edit && (
                <button aria-label={t("hotlist.delete")} title={t("hotlist.delete")} onClick={() => useHotlist.getState().remove(item.id)}>
                  <Trash2 size={12} />
                </button>
              )}
            </div>
          );
        }
        const sub = item.type === "submenu";
        const isOpen = sub && (edit || open.has(item.id));
        return (
          <div
            key={item.id}
            role="treeitem"
            aria-level={depth + 1}
            aria-expanded={sub ? isOpen : undefined}
            data-hl-id={item.id}
            data-hl-sub={sub ? "1" : undefined}
            className="mc-tree-row"
            style={{ paddingLeft: 6 + depth * 12 }}
            title={sub ? item.name : item.path}
            onPointerDown={edit ? (e) => startDrag(e, item.id) : undefined}
            // Sulla riga e non sul nome: la cattura del puntatore del trascinamento sposta il bersaglio degli eventi sulla riga.
            onDoubleClick={edit ? () => setRenaming(item.id) : undefined}
            onClick={() => {
              if (edit) return;
              if (sub) toggle(item.id);
              else openInActiveTab(item.path);
            }}
          >
            <span className="mc-tree-chev">
              {sub && !edit && (isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />)}
            </span>
            {sub ? <FolderTree size={14} className="mc-tree-icon" /> : <Folder size={14} className="mc-tree-icon" />}
            {renaming === item.id ? (
              <RenameInput
                initial={item.name}
                onDone={(name) => {
                  setRenaming(null);
                  if (name) useHotlist.getState().rename(item.id, name);
                }}
              />
            ) : (
              <span className="mc-tree-name">{item.name}</span>
            )}
            {edit && renaming !== item.id && (
              <span className="mc-tree-actions">
                <button aria-label={t("hotlist.rename")} title={t("hotlist.rename")} onClick={() => setRenaming(item.id)}>
                  <Pencil size={12} />
                </button>
                <button aria-label={t("hotlist.delete")} title={t("hotlist.delete")} onClick={() => useHotlist.getState().remove(item.id)}>
                  <Trash2 size={12} />
                </button>
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function FavoritesView() {
  const { t } = useTranslation();
  const editing = useHotlist((s) => s.editing);
  const [renaming, setRenaming] = useState<string | null>(null);
  const current = useLayout((s) => activeTab(s).path); // anche per una scheda di file è la sua cartella

  return (
    <>
      <div className="mc-side-tools">
        <button
          aria-label={t("hotlist.addCurrent")}
          title={t("hotlist.addCurrent")}
          disabled={current === DRIVES}
          onClick={() => useHotlist.getState().addDir(current, folderName(current))}
        >
          <Star size={14} />
        </button>
        {editing && (
          <button
            aria-label={t("hotlist.newSubmenu")}
            title={t("hotlist.newSubmenu")}
            onClick={() => setRenaming(useHotlist.getState().addSubmenu(t("hotlist.newSubmenu")))}
          >
            <FolderPlus size={14} />
          </button>
        )}
        <button
          className={editing ? "on" : ""}
          aria-label={editing ? t("hotlist.done") : t("hotlist.edit")}
          title={editing ? t("hotlist.done") : t("hotlist.edit")}
          aria-pressed={editing}
          onClick={() => {
            setRenaming(null);
            useHotlist.getState().setEditing(!editing);
          }}
        >
          {editing ? <Check size={14} /> : <Pencil size={14} />}
        </button>
      </div>
      <FavoritesList edit={editing} fill renaming={renaming} setRenaming={setRenaming} />
    </>
  );
}
