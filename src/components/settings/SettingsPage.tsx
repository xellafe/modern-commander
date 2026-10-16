import { Keyboard } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings, type Settings } from "../../state/settings";
import { useUi } from "../../state/ui";

type Category = "appearance" | "files" | "editor";
const CATEGORIES: Category[] = ["appearance", "files", "editor"];

type Item =
  | { id: "theme" | "language"; category: Category; kind: "select"; options: string[] }
  | { id: "showHidden" | "showSystem" | "confirmDelete"; category: Category; kind: "check" }
  | { id: "externalEditor"; category: Category; kind: "text" };

const ITEMS: Item[] = [
  { id: "theme", category: "appearance", kind: "select", options: ["system", "dark", "light"] },
  { id: "language", category: "appearance", kind: "select", options: ["en", "it"] },
  { id: "showHidden", category: "files", kind: "check" },
  { id: "showSystem", category: "files", kind: "check" },
  { id: "confirmDelete", category: "files", kind: "check" },
  { id: "externalEditor", category: "editor", kind: "text" },
];

// Le modifiche si applicano subito (tema, lingua e liste dei file reagiscono allo store, vedi main.tsx).
const save = (patch: Partial<Settings>) => useSettings.getState().update(patch).catch(console.error);

/** Editor esterno: si salva all'uscita dal campo e solo se non è vuoto. */
function EditorField({ labelId }: { labelId: string }) {
  const { t } = useTranslation();
  const stored = useSettings((s) => s.externalEditor);
  const [draft, setDraft] = useState(stored);
  const [invalid, setInvalid] = useState(false);
  const errorId = `${labelId}-error`;

  const commit = () => {
    const value = draft.trim();
    setInvalid(!value);
    if (!value) return;
    setDraft(value);
    if (value !== stored) void save({ externalEditor: value });
  };

  return (
    <>
      <input
        type="text"
        value={draft}
        spellCheck={false}
        aria-labelledby={labelId}
        aria-invalid={invalid}
        aria-describedby={invalid ? errorId : undefined}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit(); // il focus resta nel campo: Esc continua a chiudere la pagina
          }
        }}
      />
      {invalid && <div className="mc-page-error" id={errorId} role="alert">{t("settings.errors.editorEmpty")}</div>}
    </>
  );
}

export function SettingsPage({ query }: { query: string }) {
  const { t } = useTranslation();
  const settings = useSettings();
  const [category, setCategory] = useState<Category | null>(null);

  const q = query.trim().toLowerCase();
  const visible = ITEMS.filter(
    (it) =>
      (!category || it.category === category) &&
      (!q || `${t(`settings.items.${it.id}.label`)} ${t(`settings.items.${it.id}.desc`)} ${it.id}`.toLowerCase().includes(q)),
  );

  return (
    <div className="mc-page-main">
      <nav className="mc-page-cats">
        {[null, ...CATEGORIES].map((c) => (
          <button key={c ?? "all"} className={c === category ? "on" : ""} aria-pressed={c === category} onClick={() => setCategory(c)}>
            {t(`settings.categories.${c ?? "all"}`)}
          </button>
        ))}
        <button className="mc-page-link" onClick={() => useUi.getState().openPage("keybindings")}>
          <Keyboard size={13} strokeWidth={1.6} aria-hidden />
          {t("commands.mc_Keybindings")}
        </button>
      </nav>
      <div className="mc-page-scroll">
        {visible.length === 0 && <p className="mc-page-empty">{t("settings.noResults")}</p>}
        {visible.map((it) => {
          const labelId = `setting-${it.id}`;
          const label = t(`settings.items.${it.id}.label`);
          const desc = <div className="mc-setting-desc">{t(`settings.items.${it.id}.desc`)}</div>;
          return (
            <section key={it.id} className="mc-setting">
              {it.kind === "check" ? (
                <label className="mc-setting-check">
                  <input type="checkbox" checked={settings[it.id]} onChange={(e) => void save({ [it.id]: e.target.checked })} />
                  <span className="mc-setting-label" id={labelId}>{label}</span>
                </label>
              ) : (
                <div className="mc-setting-label" id={labelId}>{label}</div>
              )}
              {desc}
              {it.kind === "select" && (
                <select
                  aria-labelledby={labelId}
                  value={settings[it.id]}
                  onChange={(e) => void save({ [it.id]: e.target.value })}
                >
                  {it.options.map((o) => (
                    <option key={o} value={o}>{t(`settings.options.${it.id}.${o}`)}</option>
                  ))}
                </select>
              )}
              {it.kind === "text" && <EditorField labelId={labelId} />}
            </section>
          );
        })}
      </div>
    </div>
  );
}
