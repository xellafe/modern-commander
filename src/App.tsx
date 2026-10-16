import "./components/shell/shell.css";
import { ActivityBar } from "./components/shell/ActivityBar";
import { BottomPanel } from "./components/shell/BottomPanel";
import { FKeyBar } from "./components/shell/FKeyBar";
import { Sidebar } from "./components/shell/Sidebar";
import { StatusBar } from "./components/shell/StatusBar";
import { TitleBar } from "./components/shell/TitleBar";
import { useUi } from "./state/ui";

// Slot riempito dal Task 11 (gruppi di schede e pannelli file).
function GroupsArea() {
  return <main className="mc-groups" />;
}

export default function App() {
  const sidebarVisible = useUi((s) => s.sidebarVisible);
  const bottomPanelVisible = useUi((s) => s.bottomPanelVisible);

  return (
    <div className="mc-shell">
      <TitleBar />
      <div className="mc-main">
        <ActivityBar />
        {sidebarVisible && <Sidebar />}
        <GroupsArea />
      </div>
      {bottomPanelVisible && <BottomPanel />}
      <FKeyBar />
      <StatusBar />
    </div>
  );
}
