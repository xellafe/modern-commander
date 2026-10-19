import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { flushSync } from "react-dom";
import { useTranslation } from "react-i18next";
import { jumpItems, paletteCommands, runJump, type JumpItem, type PaletteItem } from "../../commands/palette";
import { getCommands } from "../../commands/registry";
import { cancelRequest, nextRequestId, scanTree } from "../../ipc/api";
import type { ScanEntry } from "../../ipc/types";
import { fuzzyFilter, highlightRuns, type FuzzyMatch } from "../../lib/fuzzy";
import { DRIVES } from "../../lib/paths";
import { hotlistDirs, useHotlist } from "../../state/hotlist";
import { useKeybindings } from "../../state/keybindings";
import { usePalette, type PaletteMode } from "../../state/palette";
import { useRecentDirs } from "../../state/recentDirs";
import { useSettings } from "../../state/settings";
import { activeTabContext } from "../../state/tabViews";
import { useModalFocus } from "../dialogs/focusTrap";
import "./palette.css";

const MAX_ROWS = 50;
/** Limiti della scansione del salto rapido. */
const SCAN_DEPTH = 8;
const SCAN_ENTRIES = 20_000;

type Row = { item: PaletteItem; match: FuzzyMatch };
type ScanState = "scanning" | "done" | "truncated";

/**
 * Sorgente del salto rapido: recenti e preferiti subito (fissati all'apertura), poi i file della cartella della scheda
 * attiva a lotti, man mano che la scansione li consegna. Alla chiusura la scansione si annulla. Disattiva: tutto vuoto.
 */
function useJumpSource(enabled: boolean) {
  // La cartella si fissa all'apertura: la scheda attiva non cambia sotto la palette, ma `run` deve usare questa.
  const [root] = useState(() => {
    const path = enabled ? activeTabContext().path : DRIVES;
    return path === DRIVES ? null : path; // nell'elenco delle unità non c'è niente da scandire
  });
  const [files, setFiles] = useState<ScanEntry[]>([]);
  const [scan, setScan] = useState<ScanState>(root ? "scanning" : "done");
  // La cartella aperta non si propone tra le recenti.
  const [saved] = useState(() => ({
    recent: enabled ? useRecentDirs.getState().dirs.filter((p) => p.toLowerCase() !== root?.toLowerCase()) : [],
    favorites: enabled ? hotlistDirs(useHotlist.getState().items) : [],
  }));

  useEffect(() => {
    if (!root) return;
    const id = nextRequestId();
    let live = true;
    setFiles([]);
    setScan("scanning");
    scanTree(id, root, { maxDepth: SCAN_DEPTH, maxEntries: SCAN_ENTRIES, includeHidden: useSettings.getState().showHidden }, (batch) => {
      if (live) setFiles((f) => f.concat(batch));
    }).then(
      (truncated) => live && setScan(truncated ? "truncated" : "done"),
      (e) => {
        if (!live) return;
        console.error(e);
        setScan("done"); // quanto arrivato resta utilizzabile
      },
    );
    return () => {
      live = false;
      cancelRequest(id).catch(() => {});
    };
  }, [root]);

  const items = useMemo(() => jumpItems(saved.recent, saved.favorites, files), [saved, files]);
  return { root, items, scan };
}

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
  const jump = mode === "jump";
  const source = useJumpSource(jump);
  // Le voci si fissano all'apertura: i comandi eseguibili in quel momento.
  const items = useMemo<PaletteItem[]>(
    () => (mode === "commands" ? paletteCommands(getCommands(), keymap, t) : []),
    [mode, keymap, t],
  );
  const rows = useMemo<Row[]>(() => {
    if (!jump) return fuzzyFilter(items, query, (i) => i.label, MAX_ROWS);
    const t0 = performance.now();
    const hits = fuzzyFilter(source.items, query, (i) => i.label, MAX_ROWS);
    if (import.meta.env.DEV && source.items.length >= 1000) {
      console.log(`[perf] jump ${source.items.length} voci, "${query}": fuzzyFilter ${(performance.now() - t0).toFixed(1)} ms`);
    }
    // Le righe visibili sono poche: solo per queste si costruisce la voce della palette.
    return hits.map(({ item, match }) => ({ item: jumpPaletteItem(item, source.root, t), match }));
  }, [jump, items, source.items, source.root, query, t]);
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
    // Ctrl+P a palette aperta: niente stampa del WebView (il salto rapido è già qui, o si apre dai comandi).
    if (e.ctrlKey && !e.shiftKey && e.code === "KeyP") {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
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
      <div ref={ref} className="mc-palette" role="dialog" aria-modal="true" aria-label={t(jump ? "commands.mc_QuickJump" : "commands.mc_CommandPalette")} onKeyDown={onKeyDown}>
        <input
          data-autofocus
          type="text"
          role="combobox"
          aria-expanded
          aria-controls={`${id}-list`}
          aria-activedescendant={rows.length ? `${id}-${sel}` : undefined}
          autoComplete="off"
          spellCheck={false}
          placeholder={t(jump ? "palette.placeholderJump" : "palette.placeholder")}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSel(0);
          }}
        />
        {rows.length === 0 ? (
          <div className="mc-palette-empty">{t(jump ? "palette.emptyJump" : "palette.empty")}</div>
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
        {jump && source.scan !== "done" && (
          <div className="mc-palette-status" role="status">{t(source.scan === "scanning" ? "palette.scanning" : "palette.truncated")}</div>
        )}
      </div>
    </div>
  );
}

/** Voce della palette per una voce del salto rapido; `root` = cartella scandita (null se non c'è). */
function jumpPaletteItem(item: JumpItem, root: string | null, t: (key: string) => string): PaletteItem {
  const where = item.kind === "file" ? item.rel : item.path;
  return {
    id: `${item.kind}:${item.label}:${where}`,
    label: item.label,
    detail: item.kind === "favorite" ? item.path : item.kind === "file" && item.isDir ? t("palette.folder") : undefined,
    // I file arrivano solo da una scansione, quindi `root` c'è; le recenti e i preferiti non ne hanno bisogno.
    run: () => runJump(item, root ?? ""),
  };
}
