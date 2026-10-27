// Terminale a schede nel pannello in basso: Ctrl+ò mostra/nasconde, Ctrl+Shift+ò ne apre uno nuovo.
import type { ShellKind } from "../ipc/types";
import { sourceCaps } from "../lib/paths";
import { useLayout } from "../state/layout";
import { pendingTerminals, toggleTerminal, useTerminals } from "../state/terminals";
import { useUi } from "../state/ui";
import { registerCommand } from "./registry";

/**
 * Apre un terminale nella cartella della scheda attiva del gruppo attivo. Su una scheda remota o nelle unità vale
 * l'ultima scheda locale attiva (gruppi dal più recente); senza, la cartella vuota: il backend usa quella dell'utente.
 */
export async function newTerminal(shell?: ShellKind): Promise<void> {
  const l = useLayout.getState();
  const cwd = l.activationOrder
    .map((id) => l.tabs[l.groups.find((g) => g.id === id)!.activeTabId].path)
    .find((p) => sourceCaps(p).shell) ?? "";
  useUi.getState().showBottomPanel("terminal");
  await useTerminals.getState().create(cwd, shell);
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
