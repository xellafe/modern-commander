import { useEffect, useState } from "react";
import { buildKeymap, loadKeymap } from "./commands/keymap";
import { DEFAULT_KEYMAP } from "./commands/tcKeymap";
import { useKeyboard } from "./commands/useKeyboard";
import { GroupsArea } from "./components/groups/GroupsArea";
import "./components/shell/shell.css";
import { ActivityBar } from "./components/shell/ActivityBar";
import { BottomPanel } from "./components/shell/BottomPanel";
import { FKeyBar } from "./components/shell/FKeyBar";
import { Sidebar } from "./components/shell/Sidebar";
import { StatusBar } from "./components/shell/StatusBar";
import { TitleBar } from "./components/shell/TitleBar";
import { useUi } from "./state/ui";

export default function App() {
  const sidebarVisible = useUi((s) => s.sidebarVisible);
  const bottomPanelVisible = useUi((s) => s.bottomPanelVisible);
  // Mappa di fabbrica subito, poi quella con le personalizzazioni di keybindings.json.
  const [keymap, setKeymap] = useState(() => buildKeymap(DEFAULT_KEYMAP, []));
  useEffect(() => {
    loadKeymap().then(setKeymap).catch(console.error);
  }, []);
  useKeyboard(keymap);

  return (
    <div className="mc-shell">
      <TitleBar />
      <div className="mc-main">
        <ActivityBar />
        {sidebarVisible && <Sidebar />}
        <GroupsArea keymap={keymap} />
      </div>
      {bottomPanelVisible && <BottomPanel />}
      <FKeyBar />
      <StatusBar />
    </div>
  );
}
