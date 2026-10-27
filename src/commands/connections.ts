// Gestore delle connessioni (Ctrl+F, Task 10).
import { useUi } from "../state/ui";
import { registerCommand } from "./registry";

export function registerConnectionCommands() {
  // `cm_FtpConnect` è l'id di TC per Ctrl+F; qui vale per tutti gli schemi remoti.
  registerCommand({
    id: "cm_FtpConnect",
    titleKey: "commands.cm_FtpConnect",
    when: "group",
    run: () => useUi.getState().openDialog({ kind: "connections" }),
  });
}
