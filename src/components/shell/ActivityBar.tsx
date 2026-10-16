import { FolderTree, Settings, Star, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUi, type SidebarView } from "../../state/ui";

// Cerca e Git arrivano nelle fasi successive: restano fuori finché non hanno una vista.
const VIEWS: { id: SidebarView; icon: LucideIcon }[] = [
  { id: "explorer", icon: FolderTree },
  { id: "favorites", icon: Star },
  { id: "settings", icon: Settings },
];

export function ActivityBar() {
  const { t } = useTranslation();
  const { sidebarView, sidebarVisible, setSidebarView, toggleSidebar } = useUi();

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
    </nav>
  );
}
