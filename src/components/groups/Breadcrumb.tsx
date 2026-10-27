import { ChevronRight } from "lucide-react";
import { Fragment, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { DRIVES, normalizePath, parseRemote, pathCrumbs } from "../../lib/paths";
import { isTypedAddress, resolveTypedAddress, useConnections } from "../../state/connections";
import { useTabViews } from "../../state/tabViews";
import { useUi } from "../../state/ui";
import { ConnIcon } from "./ConnIcon";

// Segmenti cliccabili; un click sullo spazio vuoto apre il campo di testo (Invio naviga, Esc annulla).
export function Breadcrumb({ tabId, path }: { tabId: string; path: string }) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const go = (p: string) => void useTabViews.getState().load(tabId, p);
  const remote = parseRemote(path);
  const connName = useConnections((s) => (remote ? s.nameOf(remote.conn) : ""));
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [editing]);

  if (editing) {
    return (
      <div className="mc-crumb">
        <input
          ref={input}
          aria-label={t("groups.path")}
          defaultValue={path === DRIVES ? "" : path}
          onBlur={() => setEditing(false)}
          onKeyDown={(e) => {
            if (e.key !== "Enter" && e.key !== "Escape") return;
            // Il campo sparisce subito: senza stopPropagation Invio arriverebbe alla lista come cm_Return.
            e.stopPropagation();
            setEditing(false);
            if (e.key !== "Enter") return;
            // Indirizzo con un host vero: la connessione salvata corrispondente, altrimenti una nuova da salvare.
            const value = e.currentTarget.value;
            const typed = resolveTypedAddress(value, useConnections.getState().list);
            if (!typed && isTypedAddress(value)) return useTabViews.getState().showNotice(tabId, { code: "INVALID_REQUEST" });
            if (typed && "create" in typed) return useUi.getState().openDialog({ kind: "connections", draft: typed.create });
            const p = typed ? typed.open : normalizePath(value);
            if (p && p !== path) go(p);
          }}
        />
      </div>
    );
  }

  const crumbs = path === DRIVES ? [{ label: t("groups.drives"), path }] : pathCrumbs(path);
  // Sul remoto il primo segmento è la connessione: il suo nome al posto dell'id.
  if (remote) crumbs[0] = { ...crumbs[0], label: connName };
  return (
    <div className="mc-crumb" onClick={(e) => e.target === e.currentTarget && setEditing(true)} title={path}>
      {remote && <ConnIcon scheme={remote.scheme} size={11} />}
      {crumbs.map((c, i) => (
        <Fragment key={c.path}>
          {i > 0 && <ChevronRight size={11} className="sep" aria-hidden />}
          {i === crumbs.length - 1
            ? <b>{c.label}</b>
            : <span className="seg" onClick={() => go(c.path)}>{c.label}</span>}
        </Fragment>
      ))}
    </div>
  );
}
