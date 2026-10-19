import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { registerFileOpCommands } from "./commands/fileOps";
import { registerHotlistCommands } from "./commands/hotlist";
import { initCloseGuard, registerNavigationCommands } from "./commands/navigation";
import { registerPaletteCommands } from "./commands/palette";
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
registerPaletteCommands();
window.addEventListener("contextmenu", suppressKeyboardContextMenu);
// Coda delle operazioni: eventi e stato attuale (dopo un ricaricamento anche i prompt in attesa).
initJobs().catch(console.error);
// Chiusura con operazioni non finite: conferma invece di ucciderle.
initCloseGuard().catch(console.error);

async function bootstrap() {
  // Letture indipendenti in parallelo (avvio < 1 s), tutte prima del render: nessuna modifica può
  // sovrascrivere un file non ancora letto.
  await Promise.all([
    useSettings.getState().load().catch(() => {}), // se fallisce l'app parte con i valori di default
    useHotlist.getState().load(),
    useKeybindings.getState().load().catch(console.error), // se fallisce restano i tasti di fabbrica
  ]);
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
