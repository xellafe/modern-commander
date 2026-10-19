import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { flushSync } from "react-dom";
import { useTranslation } from "react-i18next";
import { paletteCommands, type PaletteItem } from "../../commands/palette";
import { getCommands } from "../../commands/registry";
import { fuzzyFilter, highlightRuns } from "../../lib/fuzzy";
import { useKeybindings } from "../../state/keybindings";
import { usePalette, type PaletteMode } from "../../state/palette";
import { useModalFocus } from "../dialogs/focusTrap";
import "./palette.css";

const MAX_ROWS = 50;

/** Palette in alto al centro (come VS Code): campo con focus, elenco filtrato con i caratteri trovati in evidenza. */
export function Palette() {
  const mode = usePalette((s) => s.mode);
  return mode ? <PaletteBox key={mode} mode={mode} /> : null;
}

function PaletteBox({ mode }: { mode: PaletteMode }) {
  const { t } = useTranslation();
  const keymap = useKeybindings((s) => s.keymap);
  const id = useId();
  const ref = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  // Le voci si fissano all'apertura: i comandi eseguibili in quel momento.
  const items = useMemo<PaletteItem[]>(
    () => (mode === "commands" ? paletteCommands(getCommands(), keymap, t) : []),
    [mode, keymap, t],
  );
  const rows = useMemo(() => fuzzyFilter(items, query, (i) => i.label, MAX_ROWS), [items, query]);
  useModalFocus(ref);

  useEffect(() => {
    listRef.current?.children[sel]?.scrollIntoView({ block: "nearest" });
  }, [sel, rows]);

  const close = () => usePalette.getState().close();

  // Chiude (e restituisce il focus a chi lo aveva: useModalFocus) prima di eseguire, così il comando parte dalla lista.
  const pick = async (item: PaletteItem) => {
    flushSync(close);
    try {
      await item.run();
    } catch (e) {
      console.error(e);
    }
  };

  // Si fermano i tasti gestiti qui; gli altri arrivano a useKeyboard, che col focus nella palette (role="dialog")
  // esegue solo i comandi globali (Ctrl+Shift+P chiude, Alt+F4 esce).
  const onKeyDown = (e: KeyboardEvent) => {
    const move = (delta: number) => rows.length && setSel((sel + delta + rows.length) % rows.length);
    switch (e.key) {
      case "ArrowDown": move(1); break;
      case "ArrowUp": move(-1); break;
      case "Enter":
        if (rows[sel]) void pick(rows[sel].item);
        break;
      case "Escape": close(); break;
      case "Tab": break; // il focus resta nel campo
      default: return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div className="mc-palette-backdrop" onMouseDown={(e) => e.target === e.currentTarget && (e.preventDefault(), close())}>
      <div ref={ref} className="mc-palette" role="dialog" aria-modal="true" aria-label={t("commands.mc_CommandPalette")} onKeyDown={onKeyDown}>
        <input
          data-autofocus
          type="text"
          role="combobox"
          aria-expanded
          aria-controls={`${id}-list`}
          aria-activedescendant={rows.length ? `${id}-${sel}` : undefined}
          autoComplete="off"
          spellCheck={false}
          placeholder={t("palette.placeholder")}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSel(0);
          }}
        />
        {rows.length === 0 ? (
          <div className="mc-palette-empty">{t("palette.empty")}</div>
        ) : (
          <ul ref={listRef} id={`${id}-list`} role="listbox" aria-label={t("palette.results")}>
            {rows.map(({ item, match }, i) => (
              <li
                key={item.id}
                id={`${id}-${i}`}
                role="option"
                aria-selected={i === sel}
                className={i === sel ? "sel" : undefined}
                onMouseDown={(e) => e.preventDefault()} // il focus resta nel campo
                onMouseMove={() => i !== sel && setSel(i)}
                onClick={() => void pick(item)}
              >
                <span className="mc-palette-label">
                  {/* tratti di testo come elementi React: mai HTML da dati */}
                  {highlightRuns(item.label, match.positions).map((r, k) =>
                    r.hit ? <mark key={k}>{r.text}</mark> : <span key={k}>{r.text}</span>,
                  )}
                </span>
                {item.detail && <span className="mc-palette-detail">{item.detail}</span>}
                {item.keys?.map((chord, k) => (
                  <span key={k} className="mc-palette-chord">
                    {chord.map((part, j) => <kbd key={j}>{part}</kbd>)}
                  </span>
                ))}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
