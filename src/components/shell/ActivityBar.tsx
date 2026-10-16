import { FolderTree, Settings, Star, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUi, type SidebarView } from "../../state/ui";

// Cerca e Git arrivano nelle fasi successive: restano fuori finché non hanno una vista.
const VIEWS: { id: SidebarView; icon: LucideIcon }[] = [
  { id: "explorer", icon: FolderTree },
  { id: "favorites", icon: Star },
];

export function ActivityBar() {
  const { t } = useTranslation();
  const { sidebarView, sidebarVisible, setSidebarView, toggleSidebar, page, openPage, closePage } = useUi();

  return (
    <nav className="mc-activity">
      {VIEWS.map(({ id, icon: Icon }) => {
        const active = sidebarVisible && sidebarView === id;
        return (
          <button
            key={id}
            className={active ? "on" : ""}
            title={t(`shell.views.${id}`)}
            aria-label={t(`shell.views.${id}`)}
            aria-pressed={active}
            // Come in VS Code: un secondo clic sulla vista attiva chiude la barra laterale.
            onClick={() => (active ? toggleSidebar() : setSidebarView(id))}
          >
            <Icon size={18} strokeWidth={1.6} />
          </button>
        );
      })}
      {/* Le impostazioni non hanno una vista nella barra laterale: aprono la pagina a schermo intero. */}
      <button
        className={page === "settings" ? "end on" : "end"}
        title={t("shell.views.settings")}
        aria-label={t("shell.views.settings")}
        aria-pressed={page === "settings"}
        onClick={() => (page === "settings" ? closePage() : openPage("settings"))}
      >
        <Settings size={18} strokeWidth={1.6} />
      </button>
    </nav>
  );
}
