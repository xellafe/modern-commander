import { Eye, Pencil, Plus, SquareSplitHorizontal, X } from "lucide-react";
import type { PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { chordParts } from "../../commands/keys";
import { chordsOf } from "../../commands/keymap";
import { closeTab } from "../../commands/navigation";
import { useKeybindings } from "../../state/keybindings";
import { DRIVES, folderName, parseRemote } from "../../lib/paths";
import { useConnections } from "../../state/connections";
import { ConnIcon } from "../dialogs/ConnectionsDialog";
import { useEditors } from "../../state/editors";
import { useLayout, type Group, type Tab } from "../../state/layout";

// Trascinamento delle schede con pointer events e non con HTML5 DnD: su Windows il DnD HTML5
// funziona solo disattivando il gestore drag&drop di Tauri, che serve per il drop da Explorer.
const DROP_CLASSES = ["drop-before", "drop-after", "drop-end"];

function dropTarget(x: number, y: number) {
  const el = document.elementFromPoint(x, y);
  const groupEl = el?.closest<HTMLElement>("[data-group-id]");
  if (!groupEl) return null;
  const groupId = groupEl.dataset.groupId!;
  const tabEl = el!.closest<HTMLElement>(".mc-tab[data-index]");
  if (!tabEl) return { groupId, index: Number.MAX_SAFE_INTEGER, el: groupEl.querySelector<HTMLElement>(".mc-tabs"), cls: "drop-end" };
  const r = tabEl.getBoundingClientRect();
  const after = x > r.left + r.width / 2;
  return { groupId, index: Number(tabEl.dataset.index) + (after ? 1 : 0), el: tabEl, cls: after ? "drop-after" : "drop-before" };
}

function clearDropMarks() {
  document.querySelectorAll(".mc-tabs .drop-before, .mc-tabs .drop-after, .mc-tabs.drop-end")
    .forEach((n) => n.classList.remove(...DROP_CLASSES));
}

function startTabDrag(e: PointerEvent<HTMLElement>, tabId: string, groupId: string, index: number) {
  if (e.button !== 0) return;
  useLayout.getState().activateTab(tabId);
  const el = e.currentTarget;
  el.setPointerCapture(e.pointerId);
  const x0 = e.clientX;
  const y0 = e.clientY;
  let dragging = false;
  const move = (ev: globalThis.PointerEvent) => {
    if (!dragging && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 5) return;
    dragging = true;
    clearDropMarks();
    const t = dropTarget(ev.clientX, ev.clientY);
    t?.el?.classList.add(t.cls);
  };
  const stop = (ev: globalThis.PointerEvent) => {
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerup", stop);
    el.removeEventListener("pointercancel", stop);
    clearDropMarks();
    const t = dragging && ev.type === "pointerup" ? dropTarget(ev.clientX, ev.clientY) : null;
    if (!t) return;
    // Nello stesso gruppo l'indice si calcola sulla lista senza la scheda trascinata.
    const at = t.groupId === groupId && t.index > index ? t.index - 1 : t.index;
    useLayout.getState().moveTab(tabId, t.groupId, at);
  };
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", stop);
  el.addEventListener("pointercancel", stop);
}

export function TabStrip({ group }: { group: Group }) {
  const { t } = useTranslation();
  const tabs = useLayout((s) => s.tabs);
  const newTab = useLayout((s) => s.newTab);
  const docs = useEditors((s) => s.docs);
  const splitKeys = useKeybindings((s) => chordsOf(s.keymap, "mc_SplitGroup")[0]);
  const splitTitle = t("commands.mc_SplitGroup") + (splitKeys ? ` (${chordParts(splitKeys).join("+")})` : "");
  const nameOf = useConnections((s) => s.nameOf);
  useConnections((s) => s.list); // i nomi cambiano col gestore delle connessioni
  const label = (tab: Tab) => {
    if (tab.file) return folderName(tab.file.path);
    if (tab.path === DRIVES) return t("groups.drives");
    const r = parseRemote(tab.path);
    return r?.path === "/" ? nameOf(r.conn) : folderName(tab.path);
  };

  return (
    <div className="mc-tabs">
      {group.tabIds.map((id, i) => (
        <div
          key={id}
          data-index={i}
          className={`mc-tab${id === group.activeTabId ? " on" : ""}`}
          title={tabs[id].file?.path ?? tabs[id].path}
          onPointerDown={(e) => startTabDrag(e, id, group.id, i)}
          onMouseDown={(e) => e.button === 1 && e.preventDefault()} // niente autoscroll col tasto centrale
          onAuxClick={(e) => e.button === 1 && closeTab(id)}
        >
          {tabs[id].file && (tabs[id].file!.mode === "editor" ? <Pencil size={12} aria-hidden /> : <Eye size={12} aria-hidden />)}
          {!tabs[id].file && parseRemote(tabs[id].path) && <ConnIcon scheme={parseRemote(tabs[id].path)!.scheme} />}
          <span className="mc-tab-label">{label(tabs[id])}</span>
          {docs[id]?.dirty && <span className="mc-tab-dirty" role="img" aria-label={t("editor.unsavedTitle")} title={t("editor.unsavedTitle")} />}
          <button
            className="mc-tab-close"
            aria-label={t("groups.closeTab")}
            title={t("groups.closeTab")}
            onPointerDown={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => closeTab(id)}
          >
            <X size={12} />
          </button>
        </div>
      ))}
      <button
        className="mc-tab-add"
        aria-label={t("groups.newTab")}
        title={t("groups.newTab")}
        onMouseDown={(e) => e.preventDefault()} // non prende il focus: Spazio/Invio restano alla lista
        onClick={() => newTab(group.id, tabs[group.activeTabId].path)}
      >
        <Plus size={14} />
      </button>
      <button
        className="mc-tab-split"
        aria-label={splitTitle}
        title={splitTitle}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => useLayout.getState().splitGroup(group.id)}
      >
        <SquareSplitHorizontal size={14} />
      </button>
    </div>
  );
}
