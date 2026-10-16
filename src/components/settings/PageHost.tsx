import { Keyboard, Search, Settings, X, type LucideIcon } from "lucide-react";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { useUi, type PageKind } from "../../state/ui";
import { trapTab, useModalFocus } from "../dialogs/focusTrap";
import { KeybindingsPage } from "./KeybindingsPage";
import { SettingsPage } from "./SettingsPage";
import "./settings.css";

const TABS: { id: PageKind; icon: LucideIcon; titleKey: string; searchKey: string }[] = [
  { id: "settings", icon: Settings, titleKey: "commands.cm_Config", searchKey: "settings.searchPlaceholder" },
  { id: "keybindings", icon: Keyboard, titleKey: "commands.mc_Keybindings", searchKey: "keybindings.searchPlaceholder" },
];

/**
 * Pagina a schermo intero sopra l'area dei gruppi (Impostazioni / Scorciatoie). È una finestra modale
 * (`role="dialog"`): il focus resta dentro, Esc chiude, alla chiusura il focus torna a chi lo aveva.
 */
function Page({ kind }: { kind: PageKind }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [query, setQuery] = useState("");
  const { openPage, closePage } = useUi.getState();
  useModalFocus(ref);

  const close = () => {
    // Un campo con una modifica in sospeso (editor esterno) la salva al blur: prima di chiudere.
    (document.activeElement as HTMLElement | null)?.blur();
    closePage();
  };

  // Se il focus è finito sul body (clic fuori dai campi) Esc non passa da `onKeyDown`: lo prende la finestra.
  useEffect(() => {
    const onEscape = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) closePage();
    };
    window.addEventListener("keydown", onEscape);
    return () => window.removeEventListener("keydown", onEscape);
  }, [closePage]);

  // Come in Dialog: si fermano solo i tasti gestiti qui o dai figli (`preventDefault`); gli altri
  // arrivano a useKeyboard, che con la pagina aperta esegue solo i comandi `global`.
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.key === "Escape" || e.key === "Tab") e.stopPropagation();
    if (e.defaultPrevented) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      trapTab(e, ref.current!);
    }
  };

  const current = TABS.find((tab) => tab.id === kind)!;
  return (
    <div ref={ref} className="mc-page" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} onKeyDown={onKeyDown}>
      <header className="mc-page-head">
        <div className="mc-page-tabs" role="tablist">
          {TABS.map(({ id, icon: Icon, titleKey }) => (
            <button
              key={id}
              role="tab"
              id={id === kind ? titleId : undefined}
              aria-selected={id === kind}
              className={id === kind ? "on" : ""}
              onClick={() => {
                setQuery("");
                openPage(id);
              }}
            >
              <Icon size={14} strokeWidth={1.6} aria-hidden />
              {t(titleKey)}
            </button>
          ))}
        </div>
        <label className="mc-page-search">
          <Search size={13} aria-hidden />
          <input
            type="text"
            data-autofocus
            value={query}
            spellCheck={false}
            placeholder={t(current.searchKey)}
            aria-label={t(current.searchKey)}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
        <button className="mc-page-close" title={t("settings.close")} aria-label={t("settings.close")} onClick={close}>
          <X size={15} strokeWidth={1.6} />
        </button>
      </header>
      {kind === "settings" ? <SettingsPage query={query} /> : <KeybindingsPage query={query} />}
    </div>
  );
}

export function PageHost() {
  const page = useUi((s) => s.page);
  return page ? <Page kind={page} /> : null;
}
