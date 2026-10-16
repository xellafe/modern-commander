// Comando della hotlist di cartelle (Ctrl+D, Task 12).
import { useHotlist, type MenuAnchor } from "../state/hotlist";
import { activeTabContext } from "../state/tabViews";
import { registerCommand } from "./registry";

/** Il menu si apre sotto la riga col cursore della lista attiva; senza riga visibile, al centro del gruppo. */
function menuAnchor(): MenuAnchor {
  const { path } = activeTabContext();
  const group = document.querySelector<HTMLElement>(".mc-group.active");
  const row = group?.querySelector<HTMLElement>(".mc-row.cur");
  if (row) {
    const r = row.getBoundingClientRect();
    return { x: r.left + 12, y: r.bottom, center: false, path };
  }
  const r = (group ?? document.body).getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2, center: true, path };
}

export function registerHotlistCommands() {
  registerCommand({
    id: "cm_DirectoryHotlist",
    titleKey: "commands.cm_DirectoryHotlist",
    when: "fileList",
    run: () => useHotlist.getState().openMenu(menuAnchor()),
  });
}
