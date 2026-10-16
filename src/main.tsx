import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { registerNavigationCommands } from "./commands/navigation";
import "./styles/tokens.css";
import i18n, { initI18n } from "./i18n";
import { resolveTheme, useSettings } from "./state/settings";

const media = window.matchMedia("(prefers-color-scheme: dark)");
const applyTheme = () => {
  document.documentElement.dataset.theme = resolveTheme(useSettings.getState().theme, media.matches);
};

// Tema di default subito, prima di qualsiasi await: niente flash non stilizzato.
applyTheme();
registerNavigationCommands();

async function bootstrap() {
  // Se la lettura fallisce l'app parte comunque con i valori di default.
  await useSettings.getState().load().catch(() => {});
  await initI18n(useSettings.getState().language);
  applyTheme();
  media.addEventListener("change", applyTheme);
  useSettings.subscribe((s, prev) => {
    if (s.theme !== prev.theme) applyTheme();
    if (s.language !== prev.language) void i18n.changeLanguage(s.language);
  });

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void bootstrap();
