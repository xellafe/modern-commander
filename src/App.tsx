import { useKeyboard } from "./commands/useKeyboard";
import { DialogHost } from "./components/dialogs/Dialog";
import { HotlistMenu } from "./components/dialogs/HotlistMenu";
import { GroupsArea } from "./components/groups/GroupsArea";
import { PageHost } from "./components/settings/PageHost";
import "./components/shell/shell.css";
import { ActivityBar } from "./components/shell/ActivityBar";
import { BottomPanel } from "./components/shell/BottomPanel";
import { FKeyBar } from "./components/shell/FKeyBar";
import { Sidebar } from "./components/shell/Sidebar";
import { StatusBar } from "./components/shell/StatusBar";
import { TitleBar } from "./components/shell/TitleBar";
import { useKeybindings } from "./state/keybindings";
import { useUi } from "./state/ui";

export default function App() {
  const sidebarVisible = useUi((s) => s.sidebarVisible);
  const bottomPanelVisible = useUi((s) => s.bottomPanelVisible);
  // La mappa viva (fabbrica + keybindings.json): si aggiorna a ogni modifica dalla pagina delle scorciatoie.
  const keymap = useKeybindings((s) => s.keymap);
  useKeyboard(keymap);

  return (
    <>
      <div className="mc-shell">
        <TitleBar />
        <div className="mc-main">
          <ActivityBar />
          {sidebarVisible && <Sidebar />}
          <div className="mc-center">
            <GroupsArea keymap={keymap} />
            <PageHost />
          </div>
        </div>
        {bottomPanelVisible && <BottomPanel />}
        <FKeyBar />
        <StatusBar />
        <HotlistMenu />
      </div>
      <DialogHost />
    </>
  );
}
