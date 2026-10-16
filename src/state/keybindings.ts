import { create } from "zustand";
import { writeConfig } from "../ipc/api";
import { buildKeymap, diffOverrides, loadKeymap, resetCommand, setChords } from "../commands/keymap";
import { DEFAULT_KEYMAP } from "../commands/tcKeymap";

interface KeybindingsState {
  /** La mappa viva (chord -> id comando) che legge `useKeyboard`: si aggiorna a ogni modifica. */
  keymap: Map<string, string>;
  load: () => Promise<void>;
  /** I tasti del comando diventano esattamente `chords` (vuoto = nessun tasto). */
  setChords: (commandId: string, chords: string[]) => Promise<void>;
  reset: (commandId: string) => Promise<void>;
}

// Come per le impostazioni: le scritture partono in ordine di chiamata, l'ultima vince.
let pending: Promise<unknown> = Promise.resolve();

export const useKeybindings = create<KeybindingsState>((set, get) => {
  // La mappa cambia subito; sul file finisce solo la differenza rispetto alla mappa di fabbrica.
  const commit = (keymap: Map<string, string>) => {
    set({ keymap });
    const overrides = diffOverrides(DEFAULT_KEYMAP, keymap);
    const run = pending.then(() => writeConfig("keybindings", overrides));
    pending = run.catch(() => {}); // un errore non blocca le scritture successive
    return run;
  };
  return {
    keymap: buildKeymap(DEFAULT_KEYMAP, []),
    load: async () => set({ keymap: await loadKeymap() }),
    setChords: (id, chords) => commit(setChords(get().keymap, id, chords)),
    reset: (id) => commit(resetCommand(get().keymap, id)),
  };
});
