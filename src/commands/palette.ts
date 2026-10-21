// Comandi della palette (Ctrl+Shift+P) e del salto rapido (Ctrl+P), e le voci che la palette mostra.
import type { ScanEntry } from "../ipc/types";
import { joinPath } from "../lib/paths";
import { activeTab, useLayout } from "../state/layout";
import { usePalette } from "../state/palette";
import { useTabViews } from "../state/tabViews";
import { useUi } from "../state/ui";
import { chordParts } from "./keys";
import { chordsOf } from "./keymap";
import { ensureDirTab } from "./navigation";
import { isRunnable, registerCommand, type Command } from "./registry";

export const PALETTE_COMMAND_ID = "mc_CommandPalette";
export const QUICK_JUMP_COMMAND_ID = "mc_QuickJump";

export interface PaletteItem {
  id: string;
  label: string;
  detail?: string;
  /** Un tasto per ogni elemento, ogni tasto come elenco di parti (['Ctrl', 'Shift', 'P']). */
  keys?: string[][];
  /** `ctrl`: scelta con Ctrl+Invio o Ctrl+clic (il salto rapido la usa per la nuova scheda; i comandi la ignorano). */
  run(ctrl?: boolean): void | Promise<void>;
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

/** Una voce del salto rapido: cartella recente, cartella dei Preferiti o voce della scansione (`rel` dalla cartella scandita). */
export type JumpItem =
  | { kind: "recent" | "favorite"; path: string; label: string }
  | { kind: "file"; rel: string; isDir: boolean; label: string };

/** Prima le recenti, poi i preferiti, poi i file: `fuzzyFilter` è stabile, quindi a parità di punteggio vince quest'ordine. */
export function jumpItems(recent: string[], favorites: { name: string; path: string }[], files: ScanEntry[]): JumpItem[] {
  return [
    ...recent.map((path): JumpItem => ({ kind: "recent", path, label: path })),
    ...favorites.map((f): JumpItem => ({ kind: "favorite", path: f.path, label: f.name })),
    ...files.map((f): JumpItem => ({ kind: "file", rel: f.rel, isDir: f.isDir, label: f.rel })),
  ];
}

/** Cartella da aprire e, per un file, il nome su cui mettere il cursore. `root` = cartella che si è scandita. */
export function jumpTarget(item: JumpItem, root: string): { dir: string; focus?: string } {
  if (item.kind !== "file") return { dir: item.path };
  if (item.isDir) return { dir: joinPath(root, item.rel) };
  const cut = item.rel.lastIndexOf("\\");
  return { dir: cut < 0 ? root : joinPath(root, item.rel.slice(0, cut)), focus: item.rel.slice(cut + 1) };
}

/** Nuova scheda per la voce scelta: Ctrl (Ctrl+Invio, Ctrl+clic) inverte l'impostazione `jumpInNewTab`. */
export const wantsNewTab = (ctrl: boolean, always: boolean) => ctrl !== always;

/** Scheda in cui aprire `dir`: una nuova subito dopo quella attiva, oppure la scheda cartella del gruppo attivo. */
export function jumpTab(dir: string, newTab: boolean): string {
  if (!newTab) return ensureDirTab(dir);
  const l = useLayout.getState();
  l.newTab(l.activeGroupId, dir);
  return activeTab(useLayout.getState()).id;
}

/** Va alla voce scelta, nella scheda cartella del gruppo attivo (da un viewer/editor si torna alla lista) o in una nuova. */
export async function runJump(item: JumpItem, root: string, newTab = false): Promise<void> {
  const { dir, focus } = jumpTarget(item, root);
  await useTabViews.getState().load(jumpTab(dir, newTab), dir, { focus });
}

/** Apre il salto rapido. Con una finestra di dialogo aperta non fa nulla. */
export function quickJump(): void {
  if (!useUi.getState().dialog) usePalette.getState().open("jump");
}

export function registerPaletteCommands() {
  // `when: 'global'`: deve funzionare anche col focus in un campo o nell'editor (Monaco è una textarea).
  registerCommand({ id: PALETTE_COMMAND_ID, titleKey: `commands.${PALETTE_COMMAND_ID}`, when: "global", run: togglePalette });
  registerCommand({ id: QUICK_JUMP_COMMAND_ID, titleKey: `commands.${QUICK_JUMP_COMMAND_ID}`, when: "group", run: quickJump });
}
