import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, PanelBottom, PanelLeft, Search, Square, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUi } from "../../state/ui";

export function TitleBar() {
  const { t } = useTranslation();
  const sidebarVisible = useUi((s) => s.sidebarVisible);
  const bottomPanelVisible = useUi((s) => s.bottomPanelVisible);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const toggleBottomPanel = useUi((s) => s.toggleBottomPanel);
  const win = getCurrentWindow();

  return (
    <header className="mc-title" data-tauri-drag-region>
      <span className="mc-title-name">{t("app.title")}</span>
      <div className="mc-title-search" data-tauri-drag-region>
        <Search size={13} />
        <input disabled placeholder={t("shell.search.placeholder")} aria-label={t("shell.search.placeholder")} />
      </div>
      <div className="mc-title-toggles">
        <button
          className={sidebarVisible ? "on" : ""}
          title={t("shell.toggle.sidebar")}
          aria-label={t("shell.toggle.sidebar")}
          aria-pressed={sidebarVisible}
          onClick={toggleSidebar}
        >
          <PanelLeft size={15} />
        </button>
        <button
          className={bottomPanelVisible ? "on" : ""}
          title={t("shell.toggle.panel")}
          aria-label={t("shell.toggle.panel")}
          aria-pressed={bottomPanelVisible}
          onClick={toggleBottomPanel}
        >
          <PanelBottom size={15} />
        </button>
      </div>
      <div className="mc-title-window">
        <button title={t("shell.window.minimize")} aria-label={t("shell.window.minimize")} onClick={() => void win.minimize()}>
          <Minus size={15} />
        </button>
        <button
          title={t("shell.window.maximize")}
          aria-label={t("shell.window.maximize")}
          onClick={() => void win.toggleMaximize()}
        >
          <Square size={12} />
        </button>
        <button
          className="close"
          title={t("shell.window.close")}
          aria-label={t("shell.window.close")}
          onClick={() => void win.close()}
        >
          <X size={16} />
        </button>
      </div>
    </header>
  );
}
