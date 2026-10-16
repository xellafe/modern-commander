import { RotateCcw } from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyEvent } from "react";
import { useTranslation } from "react-i18next";
import { chordFromEvent, chordParts } from "../../commands/keys";
import { chordsOf, findConflicts, isCustomized } from "../../commands/keymap";
import { getCommand, getCommands } from "../../commands/registry";
import { useKeybindings } from "../../state/keybindings";

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "AltGraph", "Meta", "OS"]);

/** Un chord con i tasti in "keycap". */
function Keys({ chord }: { chord: string }) {
  return (
    <span className="mc-keys" title={chord}>
      {chordParts(chord).map((k, i) => (
        <Fragment key={i}>
          {i > 0 && <span className="mc-keys-plus" aria-hidden>+</span>}
          <kbd>{k}</kbd>
        </Fragment>
      ))}
    </span>
  );
}

interface Pending {
  id: string;
  chord: string;
  /** Il comando che ha già il tasto. */
  other: string;
}

/** Il prossimo tasto premuto (non un modificatore da solo) diventa il chord: Esc annulla, Backspace toglie i tasti. */
function useChordCapture(id: string | null, onChord: (chord: string) => void, onCancel: () => void, onClear: () => void) {
  const handlers = useRef({ onChord, onCancel, onClear });
  handlers.current = { onChord, onCancel, onClear };
  useEffect(() => {
    if (!id) return;
    // In cattura sulla finestra e fermando l'evento: nessun comando (nemmeno i `global`) deve partire
    // con il tasto che si sta registrando.
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.repeat || MODIFIER_KEYS.has(e.key) || e.metaKey) return;
      const bare = !e.ctrlKey && !e.shiftKey && !e.altKey;
      if (bare && e.key === "Escape") handlers.current.onCancel();
      else if (bare && e.key === "Backspace") handlers.current.onClear();
      else handlers.current.onChord(chordFromEvent(e));
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [id]);
}

export function KeybindingsPage({ query }: { query: string }) {
  const { t } = useTranslation();
  const keymap = useKeybindings((s) => s.keymap);
  const listRef = useRef<HTMLTableSectionElement>(null);
  const [active, setActive] = useState<string | null>(null);
  const [capturing, setCapturing] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);

  const title = (id: string) => {
    const cmd = getCommand(id);
    return cmd ? t(cmd.titleKey) : id;
  };

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return getCommands()
      .map((c) => ({ id: c.id, title: t(c.titleKey), chords: chordsOf(keymap, c.id), custom: isCustomized(keymap, c.id) }))
      .filter((r) => {
        if (!q) return true;
        const haystack = [r.title, r.id, ...r.chords, ...r.chords.map((c) => chordParts(c).join("+"))].join("\n");
        return haystack.toLowerCase().includes(q);
      })
      .sort((a, b) => a.title.localeCompare(b.title));
  }, [keymap, query, t]);

  const rowEl = (id: string) => listRef.current?.querySelector<HTMLElement>(`tr[data-cmd="${id}"]`);
  const focusRow = (id: string) => setTimeout(() => rowEl(id)?.focus());

  const apply = (id: string, chords: string[]) => {
    setCapturing(null);
    setPending(null);
    void useKeybindings.getState().setChords(id, chords).catch(console.error);
    focusRow(id);
  };

  useChordCapture(
    capturing,
    (chord) => {
      const other = findConflicts(keymap, chord, capturing!);
      if (other) {
        setPending({ id: capturing!, chord, other });
        setCapturing(null);
      } else apply(capturing!, [chord]);
    },
    () => {
      const id = capturing!;
      setCapturing(null);
      focusRow(id);
    },
    () => apply(capturing!, []),
  );

  // L'avviso di conflitto prende il focus su «Sostituisci» (tastiera: Invio sostituisce, Esc annulla).
  useEffect(() => {
    if (pending) listRef.current?.querySelector<HTMLElement>(".mc-conflict button.primary")?.focus();
  }, [pending]);

  const cancelPending = () => {
    const id = pending!.id;
    setPending(null);
    focusRow(id);
  };

  const onRowKeyDown = (e: ReactKeyEvent<HTMLTableRowElement>, id: string) => {
    if (e.target !== e.currentTarget) return; // i tasti del pulsante «Ripristina» restano suoi
    if (e.key === "Enter") {
      e.preventDefault();
      setPending(null);
      setCapturing(id);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const i = rows.findIndex((r) => r.id === id) + (e.key === "ArrowDown" ? 1 : -1);
      if (rows[i]) rowEl(rows[i].id)?.focus();
    }
  };

  const tabbable = rows.some((r) => r.id === active) ? active : rows[0]?.id;

  return (
    <div className="mc-page-scroll mc-kb">
      <table className="mc-kb-table">
        <thead>
          <tr>
            <th>{t("keybindings.columns.command")}</th>
            <th>{t("keybindings.columns.keys")}</th>
            <th>{t("keybindings.columns.source")}</th>
            <th />
          </tr>
        </thead>
        <tbody ref={listRef}>
          {rows.map((r) => {
            const isCapturing = capturing === r.id;
            const conflict = pending?.id === r.id ? pending : null;
            return (
              <Fragment key={r.id}>
                <tr
                  data-cmd={r.id}
                  tabIndex={r.id === tabbable ? 0 : -1}
                  className={isCapturing ? "capturing" : ""}
                  title={t("keybindings.rowHint")}
                  onFocus={(e) => e.target === e.currentTarget && setActive(r.id)}
                  onBlur={(e) => isCapturing && !e.currentTarget.contains(e.relatedTarget) && setCapturing(null)}
                  onDoubleClick={() => {
                    setPending(null);
                    setCapturing(r.id);
                  }}
                  onKeyDown={(e) => onRowKeyDown(e, r.id)}
                >
                  <td>
                    <div>{r.title}</div>
                    <div className="mc-kb-id">{r.id}</div>
                  </td>
                  <td>
                    {isCapturing ? (
                      <span className="mc-kb-capture" role="status">
                        {t("keybindings.capturePrompt")}
                        <small>{t("keybindings.captureHint")}</small>
                      </span>
                    ) : r.chords.length ? (
                      <span className="mc-kb-chords">{r.chords.map((c) => <Keys key={c} chord={c} />)}</span>
                    ) : (
                      <span className="mc-muted">{t("keybindings.unassigned")}</span>
                    )}
                  </td>
                  <td className="mc-muted">{t(`keybindings.source.${r.custom ? "user" : "default"}`)}</td>
                  <td className="mc-kb-actions">
                    {r.custom && (
                      <button
                        title={t("keybindings.resetFor", { command: r.title })}
                        aria-label={t("keybindings.resetFor", { command: r.title })}
                        onClick={() => {
                          setPending(null);
                          void useKeybindings.getState().reset(r.id).catch(console.error);
                        }}
                      >
                        <RotateCcw size={13} strokeWidth={1.6} aria-hidden />
                      </button>
                    )}
                  </td>
                </tr>
                {conflict && (
                  <tr
                    className="mc-conflict"
                    role="alert"
                    // Esc chiude l'avviso (e non la pagina).
                    onKeyDown={(e) => {
                      if (e.key !== "Escape") return;
                      e.preventDefault();
                      cancelPending();
                    }}
                  >
                    <td colSpan={4}>
                      <span>{t("keybindings.conflict", { chord: chordParts(conflict.chord).join("+"), command: title(conflict.other) })}</span>
                      <button className="primary" onClick={() => apply(conflict.id, [conflict.chord])}>{t("keybindings.replace")}</button>
                      <button onClick={cancelPending}>{t("ops.cancel")}</button>
                    </td>
                  </tr>
                )}
              </Fragment>
            );
          })}
        </tbody>
      </table>
      {rows.length === 0 && <p className="mc-page-empty">{t("keybindings.noResults")}</p>}
    </div>
  );
}
