import { Funnel, Search } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { executeCommand } from "../../commands/registry";
import { useLayout, type Group as GroupModel } from "../../state/layout";
import { useTabViews, type MenuItem } from "../../state/tabViews";
import { Editor } from "../editor/Editor";
import { Viewer } from "../viewer/Viewer";
import { Breadcrumb } from "./Breadcrumb";
import { FileList } from "./FileList";
import { TabStrip } from "./TabStrip";

const views = () => useTabViews.getState();
const closeOverlay = () => views().setOverlay(null);

export function Group({ group, active, grow }: { group: GroupModel; active: boolean; grow: number }) {
  const { t } = useTranslation();
  const tabId = group.activeTabId;
  const path = useLayout((s) => s.tabs[tabId].path);
  const isFile = useLayout((s) => !!s.tabs[tabId].file);
  const viewerPath = useLayout((s) => (s.tabs[tabId].file?.mode === "viewer" ? s.tabs[tabId].file!.path : null));
  const editorPath = useLayout((s) => (s.tabs[tabId].file?.mode === "editor" ? s.tabs[tabId].file!.path : null));
  const hasView = useTabViews((s) => tabId in s.views);
  const overlay = useTabViews((s) => (s.overlay?.tabId === tabId ? s.overlay : null));
  const notice = useTabViews((s) => (s.notice?.tabId === tabId ? s.notice : null));
  const filter = useTabViews((s) => s.views[tabId]?.filter ?? "");

  // Le schede si caricano la prima volta che diventano visibili (avvio, "+", divisione); quelle di file non hanno elenco.
  useEffect(() => {
    if (!hasView && !isFile) void views().load(tabId, path);
  }, [tabId, hasView, isFile]); // il percorso conta solo al primo caricamento: dopo lo cambia load()

  return (
    <section
      className={`mc-group${active ? " active" : ""}`}
      style={{ flexGrow: grow }}
      data-group-id={group.id}
      onMouseDown={() => {
        if (!active) useLayout.getState().activateGroup(group.id);
        if (overlay?.kind === "quickSearch") closeOverlay();
      }}
    >
      <TabStrip group={group} />
      {viewerPath && <Viewer key={tabId} tabId={tabId} path={viewerPath} />}
      {editorPath && <Editor key={tabId} tabId={tabId} path={editorPath} />}
      {!isFile && <Breadcrumb key={tabId} tabId={tabId} path={path} />}
      {!isFile && <div className="mc-group-body">
        <FileList tabId={tabId} active={active} />
        {overlay?.kind === "menu" && <PathMenu key={overlay.title} tabId={tabId} title={overlay.title} items={overlay.items} />}
        {overlay?.kind === "appMenu" && <AppMenu items={overlay.items} />}
        {overlay?.kind === "quickSearch" && (
          <div className="mc-quicksearch" role="status" aria-label={t("groups.quickSearch")}>
            <Search size={12} aria-hidden />
            {overlay.prefix}
          </div>
        )}
        {(overlay?.kind === "filter" || filter) && <QuickFilter tabId={tabId} focus={overlay?.kind === "filter"} />}
      </div>}
      {/* Anche sulle schede di file: un salvataggio fallito dell'editor (file in uso, accesso negato) arriva qui. */}
      {notice && (
        <div className="mc-notice" role="alert">
          {t(`errors.${notice.code}`)}
          {notice.path && <span>{notice.path}</span>}
        </div>
      )}
    </section>
  );
}

// Filtro rapido (Ctrl+S): si aggiorna mentre si scrive; Invio torna alla lista, Esc azzera il filtro.
function QuickFilter({ tabId, focus }: { tabId: string; focus: boolean }) {
  const { t } = useTranslation();
  const filter = useTabViews((s) => s.views[tabId]?.filter ?? "");
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (focus) ref.current?.focus();
  }, [focus]);
  const setFilter = (f: string) => views().rebuild(tabId, { filter: f });

  return (
    <div className="mc-quickfilter">
      <Funnel size={12} aria-hidden />
      <input
        ref={ref}
        value={filter}
        aria-label={t("groups.filter")}
        placeholder={t("groups.filterPlaceholder")}
        onChange={(e) => setFilter(e.target.value)}
        onBlur={closeOverlay}
        onKeyDown={(e) => {
          if (e.key !== "Escape" && e.key !== "Enter") return;
          e.stopPropagation(); // dopo il blur Invio non deve arrivare alla lista come cm_Return
          if (e.key === "Escape") setFilter("");
          e.currentTarget.blur();
        }}
      />
    </div>
  );
}

// Tasti dei menu a comparsa: frecce, Home/End, Invio sulla voce `sel`, Esc.
function menuKeys(n: number, sel: number, setSel: (i: number) => void, choose: (i: number) => void) {
  return (e: KeyboardEvent) => {
    e.stopPropagation(); // i tasti restano al menu
    const moves: Record<string, number> = { ArrowUp: sel - 1, ArrowDown: sel + 1, Home: 0, End: n - 1 };
    if (e.key in moves) setSel(Math.max(0, Math.min(moves[e.key], n - 1)));
    else if (e.key === "Enter" && n) choose(sel);
    else if (e.key === "Escape") closeOverlay();
    else return;
    e.preventDefault();
  };
}

// Menu delle unità (Alt+F1) e della history (Alt+↓): frecce, Invio, Esc, mouse.
function PathMenu({ tabId, title, items }: { tabId: string; title: string; items: MenuItem[] }) {
  const [sel, setSel] = useState(() => Math.max(0, items.findIndex((it) => it.current)));
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => ref.current?.focus(), []);

  const choose = (it: MenuItem) => {
    closeOverlay();
    void views().load(tabId, it.path, it.historyIndex === undefined ? undefined : { historyIndex: it.historyIndex });
  };
  const onKeyDown = menuKeys(items.length, sel, setSel, (i) => choose(items[i]));

  return (
    <div ref={ref} className="mc-menu" role="dialog" aria-label={title} tabIndex={-1} onKeyDown={onKeyDown} onBlur={closeOverlay}>
      <div className="mc-menu-title">{title}</div>
      <div role="menu">
        {items.map((it, i) => (
          <div
            key={`${it.path}-${it.historyIndex ?? ""}`}
            role="menuitem"
            className={`mc-menu-item${i === sel ? " sel" : ""}${it.current ? " current" : ""}`}
            onMouseEnter={() => setSel(i)}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => choose(it)}
          >
            <span>{it.label}</span>
            {it.detail && <span className="detail">{it.detail}</span>}
          </div>
        ))}
      </div>
    </div>
  );
}

// Menu contestuale dell'app sulle schede remote (niente menu di Windows): ogni voce esegue un comando.
function AppMenu({ items }: { items: { commandId: string; labelKey: string }[] }) {
  const { t } = useTranslation();
  const [sel, setSel] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => ref.current?.focus(), []);

  const choose = (i: number) => {
    closeOverlay();
    void executeCommand(items[i].commandId);
  };
  const title = t("commands.cm_ContextMenu");

  return (
    <div ref={ref} className="mc-menu" role="dialog" aria-label={title} tabIndex={-1} onKeyDown={menuKeys(items.length, sel, setSel, choose)} onBlur={closeOverlay}>
      <div role="menu">
        {items.map((it, i) => (
          <div
            key={it.commandId}
            role="menuitem"
            className={`mc-menu-item${i === sel ? " sel" : ""}`}
            onMouseEnter={() => setSel(i)}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => choose(i)}
          >
            <span>{t(it.labelKey)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
