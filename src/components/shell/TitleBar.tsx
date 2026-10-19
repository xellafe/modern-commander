import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, PanelBottom, PanelLeft, Search, Square, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { chordParts } from "../../commands/keys";
import { chordsOf } from "../../commands/keymap";
import { PALETTE_COMMAND_ID } from "../../commands/palette";
import { executeCommand } from "../../commands/registry";
import { useKeybindings } from "../../state/keybindings";
import { useUi } from "../../state/ui";

export function TitleBar() {
  const { t } = useTranslation();
  const sidebarVisible = useUi((s) => s.sidebarVisible);
  const bottomPanelVisible = useUi((s) => s.bottomPanelVisible);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const toggleBottomPanel = useUi((s) => s.toggleBottomPanel);
  const keymap = useKeybindings((s) => s.keymap);
  const paletteKeys = chordsOf(keymap, PALETTE_COMMAND_ID)[0];
  const win = getCurrentWindow();

  return (
    <header className="mc-title" data-tauri-drag-region>
      <span className="mc-title-name">{t("app.title")}</span>
      {/* Con l'aspetto di un campo, ma è un pulsante: il click apre la palette (il focus resta dov'era). */}
      <button
        className="mc-title-search"
        aria-haspopup="dialog"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => void executeCommand(PALETTE_COMMAND_ID)}
      >
        <Search size={13} />
        <span>{t("shell.search.placeholder")}</span>
        {paletteKeys && <kbd>{chordParts(paletteKeys).join("+")}</kbd>}
      </button>
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
