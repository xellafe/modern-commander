import { useTranslation } from "react-i18next";
import { useUi } from "../../state/ui";
import { ExplorerView } from "../sidebar/ExplorerView";
import { FavoritesView } from "../sidebar/FavoritesView";
import "../sidebar/sidebar.css";
import { Splitter } from "./Splitter";

export function Sidebar() {
  const { t } = useTranslation();
  const view = useUi((s) => s.sidebarView);
  const width = useUi((s) => s.sidebarWidth);
  const setWidth = useUi((s) => s.setSidebarWidth);

  return (
    <aside className="mc-sidebar" style={{ width }}>
      <div className="mc-sidebar-head">{t(`shell.views.${view}`)}</div>
      <div className="mc-sidebar-body">
        {view === "explorer" && <ExplorerView />}
        {view === "favorites" && <FavoritesView />}
      </div>
      <Splitter axis="x" size={width} onResize={setWidth} />
    </aside>
  );
}
