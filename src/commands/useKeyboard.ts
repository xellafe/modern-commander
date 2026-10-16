import { useEffect } from 'react';
import { chordFromEvent } from './keys';
import { executeCommand, getCommand, shouldExecute } from './registry';

// Focus in un campo di testo o dentro una finestra di dialogo (o con una pagina aperta, anche se il focus
// è finito sul body): solo i comandi 'global'.
export function focusInTextOrDialog(): boolean {
  if (document.querySelector(".mc-page")) return true;
  const el = document.activeElement;
  if (!(el instanceof HTMLElement)) return false;
  return el.matches('input, textarea, select, [contenteditable]:not([contenteditable="false"])') || el.closest('[role="dialog"]') !== null;
}

export function useKeyboard(keymap: Map<string, string>): void {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const id = keymap.get(chordFromEvent(e));
      const cmd = id === undefined ? undefined : getCommand(id);
      const focusInText = focusInTextOrDialog();
      // nessun binding, comando non registrato o focus in un campo: lascia l'evento al browser
      if (!cmd || !shouldExecute(cmd, { repeat: false, focusInText })) return;
      e.preventDefault(); // anche Alt+F4, che deve passare da cm_Exit
      // l'auto-repeat è consumato ma non riesegue il comando, salvo opt-in
      if (shouldExecute(cmd, { repeat: e.repeat, focusInText })) executeCommand(cmd.id).catch(console.error);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [keymap]);
}
