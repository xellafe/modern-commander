import { ChevronDown, Plus, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { newTerminal } from "../../commands/terminal";
import { termShells } from "../../ipc/api";
import type { ShellKind } from "../../ipc/types";
import { applyTerminalTheme, layoutTerminals, setTerminalHost, showActiveTerminal, useTerminals } from "../../state/terminals";
import { useUi } from "../../state/ui";
import "./panel.css";

const report = (e: unknown) => console.error(e);

/** Striscia delle sessioni e, sotto, l'area delle istanze xterm: restano tutte montate, si vede solo l'attiva. */
export function TerminalView() {
  const { t } = useTranslation();
  const sessions = useTerminals((s) => s.sessions);
  const activeId = useTerminals((s) => s.activeId);
  const shown = useUi((s) => s.bottomPanelVisible && s.bottomPanelTab === "terminal");
  const host = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<ShellKind[] | null>(null);

  useEffect(() => {
    const el = host.current!;
    setTerminalHost(el);
    const resize = new ResizeObserver(layoutTerminals); // fit() e, se cambiano le dimensioni, termResize
    resize.observe(el);
    // Il tema cambia `data-theme` sulla radice: le istanze si ricolorano dai token.
    const theme = new MutationObserver(applyTerminalTheme);
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      resize.disconnect();
      theme.disconnect();
      setTerminalHost(null);
    };
  }, []);

  useEffect(() => showActiveTerminal(false), [sessions]);
  // Il focus passa al terminale quando lo si mostra o si cambia scheda, non a ogni modifica dell'elenco.
  useEffect(() => showActiveTerminal(shown), [activeId, shown]);

  const openMenu = () => {
    if (menu) return setMenu(null);
    termShells().then(setMenu).catch(report);
  };

  return (
    <div className="mc-term-view">
      <div className="mc-tabs" role="tablist">
        {sessions.map((s) => (
          <div key={s.id} role="tab" aria-selected={s.id === activeId} className={`mc-tab${s.id === activeId ? " on" : ""}`} onClick={() => useTerminals.getState().activate(s.id)}>
            <span className="mc-tab-label">{[t(`settings.options.terminalShell.${s.shell}`), s.title].filter(Boolean).join(" · ")}</span>
            <button
              className="mc-tab-close"
              aria-label={t("terminal.close")}
              title={t("terminal.close")}
              onClick={(e) => {
                e.stopPropagation();
                useTerminals.getState().close(s.id);
              }}
            >
              <X size={12} />
            </button>
          </div>
        ))}
        <div className="mc-term-new" onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && setMenu(null)}>
          <button className="mc-tab-add" aria-label={t("terminal.new")} title={t("terminal.new")} onClick={() => newTerminal().catch(report)}>
            <Plus size={14} />
          </button>
          <button className="mc-tab-add" aria-label={t("terminal.shellMenu")} title={t("terminal.shellMenu")} aria-haspopup="menu" aria-expanded={menu !== null} onClick={openMenu}>
            <ChevronDown size={14} />
          </button>
          {menu && (
            <div className="mc-menu mc-term-menu" role="menu" onKeyDown={(e) => e.key === "Escape" && setMenu(null)}>
              {menu.map((k) => (
                <button
                  key={k}
                  role="menuitem"
                  className="mc-menu-item"
                  onClick={() => {
                    setMenu(null);
                    newTerminal(k).catch(report);
                  }}
                >
                  {t(`settings.options.terminalShell.${k}`)}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
      {!sessions.length && <div className="mc-ops-empty">{t("terminal.empty")}</div>}
      <div className="mc-term-host" ref={host} />
    </div>
  );
}
