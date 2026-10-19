// Comando della palette (Ctrl+Shift+P) e l'elenco di voci che la palette mostra.
import { usePalette } from "../state/palette";
import { useUi } from "../state/ui";
import { chordParts } from "./keys";
import { chordsOf } from "./keymap";
import { isRunnable, registerCommand, type Command } from "./registry";

export const PALETTE_COMMAND_ID = "mc_CommandPalette";

export interface PaletteItem {
  id: string;
  label: string;
  detail?: string;
  /** Un tasto per ogni elemento, ogni tasto come elenco di parti (['Ctrl', 'Shift', 'P']). */
  keys?: string[][];
  run(): void | Promise<void>;
}

/** I comandi eseguibili ora (la palette stessa esclusa), col titolo tradotto e i tasti della mappa viva. */
export function paletteCommands(commands: Command[], keymap: Map<string, string>, t: (key: string) => string): PaletteItem[] {
  return commands
    .filter((c) => c.id !== PALETTE_COMMAND_ID && isRunnable(c))
    .map((c) => ({
      id: c.id,
      label: t(c.titleKey),
      keys: chordsOf(keymap, c.id).map(chordParts),
      run: c.run,
    }));
}

/** Apre la palette dei comandi, o la chiude se è già aperta. Con una finestra di dialogo aperta non fa nulla. */
export function togglePalette(): void {
  const { mode, open, close } = usePalette.getState();
  if (mode) close();
  else if (!useUi.getState().dialog) open("commands");
}

export function registerPaletteCommands() {
  // `when: 'global'`: deve funzionare anche col focus in un campo o nell'editor (Monaco è una textarea).
  registerCommand({ id: PALETTE_COMMAND_ID, titleKey: `commands.${PALETTE_COMMAND_ID}`, when: "global", run: togglePalette });
}
