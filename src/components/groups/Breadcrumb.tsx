import { ChevronRight } from "lucide-react";
import { Fragment, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { DRIVES, normalizePath, pathCrumbs } from "../../lib/paths";
import { useTabViews } from "../../state/tabViews";

// Segmenti cliccabili; un click sullo spazio vuoto apre il campo di testo (Invio naviga, Esc annulla).
export function Breadcrumb({ tabId, path }: { tabId: string; path: string }) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const go = (p: string) => void useTabViews.getState().load(tabId, p);
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
            const p = normalizePath(e.currentTarget.value);
            if (e.key === "Enter" && p && p !== path) go(p);
          }}
        />
      </div>
    );
  }

  const crumbs = path === DRIVES ? [{ label: t("groups.drives"), path }] : pathCrumbs(path);
  return (
    <div className="mc-crumb" onClick={(e) => e.target === e.currentTarget && setEditing(true)} title={path}>
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
