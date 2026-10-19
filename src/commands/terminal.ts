// Terminale a schede nel pannello in basso: Ctrl+ò mostra/nasconde, Ctrl+Shift+ò ne apre uno nuovo.
import type { ShellKind } from "../ipc/types";
import { DRIVES } from "../lib/paths";
import { activeTab, useLayout } from "../state/layout";
import { pendingTerminals, toggleTerminal, useTerminals } from "../state/terminals";
import { useUi } from "../state/ui";
import { registerCommand } from "./registry";

/** Apre un terminale nella cartella della scheda attiva del gruppo attivo (`drives:` passa vuota: il backend usa la cartella utente). */
export async function newTerminal(shell?: ShellKind): Promise<void> {
  const { path } = activeTab(useLayout.getState());
  useUi.getState().showBottomPanel("terminal");
  await useTerminals.getState().create(path === DRIVES ? "" : path, shell);
}

/** Mostra il pannello sul terminale (aprendone uno se non ce n'è) quando se ne sceglie la scheda. */
export function showTerminalTab(): void {
  useUi.getState().showBottomPanel("terminal");
  if (!useTerminals.getState().sessions.length && !pendingTerminals()) newTerminal().catch(console.error);
}

function run() {
  const ui = useUi.getState();
  const r = toggleTerminal({ visible: ui.bottomPanelVisible, tab: ui.bottomPanelTab }, useTerminals.getState().sessions.length > 0, pendingTerminals());
  useUi.setState({ bottomPanelVisible: r.visible, bottomPanelTab: r.tab });
  if (r.create) newTerminal().catch(console.error);
}

// `global`: devono funzionare anche col focus nel terminale (la textarea di xterm è un campo di testo) o in Monaco.
export function registerTerminalCommands() {
  registerCommand({ id: "mc_ToggleTerminal", titleKey: "commands.mc_ToggleTerminal", when: "global", run });
  registerCommand({ id: "mc_NewTerminal", titleKey: "commands.mc_NewTerminal", when: "global", run: () => newTerminal().catch(console.error) });
}
