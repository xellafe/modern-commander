// Caricamento pigro di xterm.js: il pacchetto, l'addon e il CSS restano in chunk separati, fuori da quello principale.
import type { FitAddon } from "@xterm/addon-fit";
import type { ITheme, Terminal } from "@xterm/xterm";

export interface Xterm {
  Terminal: typeof Terminal;
  FitAddon: typeof FitAddon;
}

export const TERM_FONT_SIZE = 12;

let loading: Promise<Xterm> | undefined;

/** xterm.js, importato una volta sola (il CSS arriva col suo chunk). */
export function loadXterm(): Promise<Xterm> {
  return (loading ??= Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit"), import("@xterm/xterm/css/xterm.css")])
    .then(([xterm, fit]) => ({ Terminal: xterm.Terminal, FitAddon: fit.FitAddon }))
    .catch((e) => {
      loading = undefined; // si riprova alla prossima apertura
      throw e;
    }));
}

const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

/** Colori dai token `--mc-*` del tema corrente (`data-theme`); la palette ANSI resta quella di xterm. */
export const termTheme = (): ITheme => ({
  background: token("--mc-panel"),
  foreground: token("--mc-text"),
  cursor: token("--mc-accent"),
  selectionBackground: token("--mc-selection"),
  selectionInactiveBackground: token("--mc-selection-inactive"),
});

export const termFont = (): string => token("--mc-font-mono");
