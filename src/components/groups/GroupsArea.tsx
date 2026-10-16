import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Fragment, useEffect, useRef, useState, type PointerEvent } from "react";
import { dropOnGroup } from "../../commands/fileOps";
import { chordFromEvent, quickSearchChar } from "../../commands/keys";
import { focusInTextOrDialog } from "../../commands/useKeyboard";
import { onFsChanged } from "../../ipc/api";
import { isUnwatched, useDrives } from "../../state/drives";
import { useLayout } from "../../state/layout";
import { quickSearch } from "../../state/tabView";
import { activeTabContext, useTabViews } from "../../state/tabViews";
import { Group } from "./Group";
import "./groups.css";

const MIN_GROUP_WIDTH = 240;

// Ricerca rapida: lettere (e Ctrl+Alt+lettera di TC) spostano il cursore sul primo nome col prefisso.
// Ascolta in cattura, prima di useKeyboard, così con il popup aperto Backspace/Esc restano qui.
function useQuickSearch(keymap: Map<string, string>) {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (focusInTextOrDialog()) return;
      const s = useTabViews.getState();
      const { tabId, view } = activeTabContext();
      const open = s.overlay?.kind === "quickSearch" && s.overlay.tabId === tabId ? s.overlay : null;
      const consume = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      if (open && (e.key === "Escape" || e.key === "Backspace")) {
        consume();
        const prefix = e.key === "Backspace" ? open.prefix.slice(0, -1) : "";
        s.setOverlay(prefix ? { ...open, prefix } : null);
        return;
      }
      const ch = keymap.has(chordFromEvent(e)) ? null : quickSearchChar(e);
      if (ch === null) {
        if (open) s.setOverlay(null); // un altro tasto chiude il popup e fa il suo lavoro
        return;
      }
      consume();
      const prefix = (open?.prefix ?? "") + ch;
      const i = quickSearch(view.rows, prefix, view.cursor);
      if (i < 0) return; // nessun nome con questo prefisso: il prefisso non cresce (come TC)
      s.update(tabId, (v) => ({ ...v, cursor: i }));
      s.setOverlay({ kind: "quickSearch", tabId, prefix });
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [keymap]);
}

export function GroupsArea({ keymap }: { keymap: Map<string, string> }) {
  const groups = useLayout((s) => s.groups);
  const activeGroupId = useLayout((s) => s.activeGroupId);
  // Peso flex-grow per gruppo; dopo un trascinamento vale la larghezza in px (le proporzioni restano al resize).
  const [weights, setWeights] = useState<Record<string, number>>({});
  const ref = useRef<HTMLElement>(null);
  useQuickSearch(keymap);

  useEffect(() => {
    const unlisten = onFsChanged((tabId) => void useTabViews.getState().reload(tabId));
    // Le unità rimovibili non hanno watcher: le schede visibili su di loro si rileggono al ritorno del focus.
    const onFocus = () => {
      const { groups, tabs } = useLayout.getState();
      for (const g of groups) {
        if (isUnwatched(tabs[g.activeTabId].path, useDrives.getState().drives)) void useTabViews.getState().reload(g.activeTabId);
      }
    };
    window.addEventListener("focus", onFocus);
    return () => {
      void unlisten.then((off) => off());
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // Drop da Explorer: finestra di copia verso il gruppo sotto il puntatore (posizione in pixel fisici).
  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent(({ payload: p }) => {
      if (p.type !== "drop") return;
      const dpr = window.devicePixelRatio || 1;
      const el = document.elementFromPoint(p.position.x / dpr, p.position.y / dpr);
      const groupId = el?.closest<HTMLElement>("[data-group-id]")?.dataset.groupId;
      if (groupId) dropOnGroup(groupId, p.paths).catch(console.error);
    });
    return () => void unlisten.then((off) => off());
  }, []);

  const known = groups.map((g) => weights[g.id]).filter((w) => w !== undefined);
  const fallback = known.length ? known.reduce((a, b) => a + b, 0) / known.length : 1;

  const startResize = (e: PointerEvent<HTMLDivElement>, i: number) => {
    const sections = [...ref.current!.querySelectorAll<HTMLElement>(":scope > .mc-group")];
    const widths = sections.map((s) => s.getBoundingClientRect().width);
    const x0 = e.clientX;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const move = (ev: globalThis.PointerEvent) => {
      const lo = MIN_GROUP_WIDTH - widths[i - 1];
      const hi = widths[i] - MIN_GROUP_WIDTH;
      const d = Math.max(lo, Math.min(ev.clientX - x0, hi));
      setWeights(Object.fromEntries(groups.map((g, k) =>
        [g.id, k === i - 1 ? widths[k] + d : k === i ? widths[k] - d : widths[k]])));
    };
    const stop = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", stop);
      el.removeEventListener("pointercancel", stop);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", stop);
    el.addEventListener("pointercancel", stop);
  };

  return (
    <main className="mc-groups" ref={ref}>
      {groups.map((g, i) => (
        <Fragment key={g.id}>
          {i > 0 && <div className="mc-group-sep" onPointerDown={(e) => startResize(e, i)} />}
          <Group group={g} active={g.id === activeGroupId} grow={weights[g.id] ?? fallback} />
        </Fragment>
      ))}
    </main>
  );
}
