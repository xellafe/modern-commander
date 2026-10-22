import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { registerConnectionCommands } from "./commands/connections";
import { registerFileOpCommands } from "./commands/fileOps";
import { registerHotlistCommands } from "./commands/hotlist";
import { LAYOUT_VKS, setLayoutLabels } from "./commands/keys";
import { keyLabels } from "./ipc/api";
import { initCloseGuard, registerNavigationCommands } from "./commands/navigation";
import { registerPaletteCommands } from "./commands/palette";
import { registerSettingsCommands } from "./commands/settings";
import { registerShellCommands, suppressNativeContextMenu } from "./commands/shell";
import { registerTerminalCommands } from "./commands/terminal";
import "./styles/tokens.css";
import i18n, { initI18n } from "./i18n";
import { useConnections } from "./state/connections";
import { blockUnloadIfDirty } from "./state/editors";
import { useHotlist } from "./state/hotlist";
import { initJobs } from "./state/jobs";
import { useKeybindings } from "./state/keybindings";
import { resolveTheme, useSettings } from "./state/settings";
import { requestTypes, useTabViews } from "./state/tabViews";

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
registerTerminalCommands();
registerConnectionCommands();
window.addEventListener("contextmenu", (e) => suppressNativeContextMenu(e, import.meta.env.DEV));
window.addEventListener("beforeunload", blockUnloadIfDirty);
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
    useConnections.getState().load().catch(console.error), // nomi delle connessioni nelle schede remote; se fallisce, gli id
    keyLabels(LAYOUT_VKS).then(setLayoutLabels).catch(() => {}), // se fallisce restano le etichette americane
  ]);
  await initI18n(useSettings.getState().language);
  applyTheme();
  media.addEventListener("change", applyTheme);
  useSettings.subscribe((s, prev) => {
    if (s.theme !== prev.theme) applyTheme();
    if (s.language !== prev.language) void i18n.changeLanguage(s.language);
    // File nascosti/di sistema: tutte le schede aperte si ricostruiscono subito dal listing in memoria.
    if (s.showHidden !== prev.showHidden || s.showSystem !== prev.showSystem) useTabViews.getState().rebuildAll();
    // Colonna Tipo accesa con schede già caricate: i testi si chiedono subito.
    if (s.columns !== prev.columns || s.extInName !== prev.extInName) {
      for (const tabId of Object.keys(useTabViews.getState().views)) void requestTypes(tabId);
    }
  });

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void bootstrap();
