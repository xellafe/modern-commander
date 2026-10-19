import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { focusInTextOrDialog } from "../../commands/useKeyboard";
import { executeCommand, getCommand, getCommands, shouldExecute } from "../../commands/registry";
import { fkeyLabels, type Mods } from "../../lib/fkeys";
import { activeTab, useLayout } from "../../state/layout";

const NO_MODS: Mods = { shift: false, ctrl: false, alt: false };

export function FKeyBar() {
  const { t } = useTranslation();
  const [mods, setMods] = useState(NO_MODS);

  useEffect(() => {
    const sync = (e: KeyboardEvent) =>
      setMods((m) => (m.shift === e.shiftKey && m.ctrl === e.ctrlKey && m.alt === e.altKey ? m : { shift: e.shiftKey, ctrl: e.ctrlKey, alt: e.altKey }));
    // Alt+Tab: il keyup dell'Alt non arriva alla finestra, senza questo il livello resterebbe bloccato.
    const reset = () => setMods(NO_MODS);
    // In cattura: i gestori React (campo di rinomina, breadcrumb...) fermano la propagazione e il livello resterebbe bloccato.
    const capture = { capture: true };
    window.addEventListener("keydown", sync, capture);
    window.addEventListener("keyup", sync, capture);
    window.addEventListener("blur", reset);
    return () => {
      window.removeEventListener("keydown", sync, capture);
      window.removeEventListener("keyup", sync, capture);
      window.removeEventListener("blur", reset);
    };
  }, []);

  const slots = fkeyLabels(mods, new Set(getCommands().map((c) => c.id)));
  return (
    <footer className="mc-fkeys">
      {slots.map((s) => (
        <button
          key={s.key}
          type="button"
          className="mc-fkey"
          aria-label={`${s.key} ${t(s.labelKey)}`}
          // La lista file tiene il fuoco: il clic non deve portarglielo via.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            // Stessa guardia della tastiera: con il fuoco in un campo o in una finestra solo i comandi globali.
            if (shouldExecute(getCommand(s.commandId), { repeat: false, focusInText: focusInTextOrDialog(), dirTab: !activeTab(useLayout.getState()).file })) {
              executeCommand(s.commandId).catch(console.error);
            }
          }}
        >
          <b>{s.key}</b>
          {t(s.labelKey)}
        </button>
      ))}
    </footer>
  );
}
