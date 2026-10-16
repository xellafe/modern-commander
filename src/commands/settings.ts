// Comandi che aprono le pagine a schermo intero (Task 21).
import { useUi, type PageKind } from "../state/ui";
import { registerCommand } from "./registry";

// Con una finestra di dialogo aperta (copia, elimina…) le pagine non si aprono sopra di lei.
const open = (page: PageKind) => () => {
  if (!useUi.getState().dialog) useUi.getState().openPage(page);
};

export function registerSettingsCommands() {
  // `cm_Config` è l'id ufficiale di TC (Ctrl+,); `mc_Keybindings` non ha un tasto: si apre dalla pagina delle impostazioni.
  registerCommand({ id: "cm_Config", titleKey: "commands.cm_Config", when: "global", run: open("settings") });
  registerCommand({ id: "mc_Keybindings", titleKey: "commands.mc_Keybindings", when: "global", run: open("keybindings") });
}
