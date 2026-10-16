import { useEffect } from 'react';
import { chordFromEvent } from './keys';
import { executeCommand, getCommand } from './registry';

// Focus in un campo di testo o dentro una finestra di dialogo: solo i comandi 'global'.
function focusInTextOrDialog(): boolean {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement)) return false;
  return el.matches('input, textarea, select, [contenteditable]:not([contenteditable="false"])') || el.closest('[role="dialog"]') !== null;
}

export function useKeyboard(keymap: Map<string, string>): void {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const id = keymap.get(chordFromEvent(e));
      const cmd = id === undefined ? undefined : getCommand(id);
      if (!cmd) return; // nessun binding o comando non registrato: lascia l'evento al browser
      if (cmd.when !== 'global' && focusInTextOrDialog()) return;
      e.preventDefault(); // anche Alt+F4, che deve passare da cm_Exit
      executeCommand(cmd.id).catch(console.error);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [keymap]);
}
