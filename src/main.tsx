import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { registerFileOpCommands } from "./commands/fileOps";
import { registerHotlistCommands } from "./commands/hotlist";
import { registerNavigationCommands } from "./commands/navigation";
import { registerSettingsCommands } from "./commands/settings";
import { registerShellCommands, suppressKeyboardContextMenu } from "./commands/shell";
import "./styles/tokens.css";
import i18n, { initI18n } from "./i18n";
import { useHotlist } from "./state/hotlist";
import { initJobs } from "./state/jobs";
import { useKeybindings } from "./state/keybindings";
import { resolveTheme, useSettings } from "./state/settings";
import { useTabViews } from "./state/tabViews";

const media = window.matchMedia("(prefers-color-scheme: dark)");
const applyTheme = () => {
  document.documentElement.dataset.theme = resolveTheme(useSettings.getState().theme, media.matches);
};

// Tema di default subito, prima di qualsiasi await: niente flash non stilizzato.
applyTheme();
registerNavigationCommands();
registerHotlistCommands();
registerFileOpCommands();
registerShellCommands();
registerSettingsCommands();
window.addEventListener("contextmenu", suppressKeyboardContextMenu);
// Coda delle operazioni: eventi e stato attuale (dopo un ricaricamento anche i prompt in attesa).
initJobs().catch(console.error);

async function bootstrap() {
  // Se la lettura fallisce l'app parte comunque con i valori di default.
  await useSettings.getState().load().catch(() => {});
  await useHotlist.getState().load(); // prima del render: nessuna modifica può sovrascrivere un file non ancora letto
  await useKeybindings.getState().load().catch(console.error); // se fallisce restano i tasti di fabbrica
  await initI18n(useSettings.getState().language);
  applyTheme();
  media.addEventListener("change", applyTheme);
  useSettings.subscribe((s, prev) => {
    if (s.theme !== prev.theme) applyTheme();
    if (s.language !== prev.language) void i18n.changeLanguage(s.language);
    // File nascosti/di sistema: tutte le schede aperte si ricostruiscono subito dal listing in memoria.
    if (s.showHidden !== prev.showHidden || s.showSystem !== prev.showSystem) useTabViews.getState().rebuildAll();
  });

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void bootstrap();
