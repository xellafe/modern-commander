import { ChevronDown } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { forgetHost, forgetPassword, importConnections, winscpCandidates } from "../../ipc/api";
import type { AppError, Connection, ConnectionView, ImportCandidate, Scheme } from "../../ipc/types";
import { defaultPort, startLocation, useConnections, validHost } from "../../state/connections";
import { activeTab, useLayout } from "../../state/layout";
import { useTabViews } from "../../state/tabViews";
import { useUi, type DialogRequest } from "../../state/ui";
import { ConnIcon } from "../groups/ConnIcon";
import { Dialog } from "./Dialog";

const SCHEMES: Scheme[] = ["sftp", "ftp", "ftps", "http", "https"];

/** Connessione in modifica; la password scritta qui resta nello stato del dialogo e si salva con "ricorda". */
type Draft = Connection & { password: string };

const blank = (c: Partial<Connection>): Draft => ({
  id: "", name: "", scheme: "sftp", host: "", port: 22, user: "", startDir: "/", ftpsImplicit: false, ...c, password: "",
});

/** Cartella iniziale nella scheda attiva, oppure in una scheda nuova se quella attiva è un file. */
function openStart(c: Connection) {
  const l = useLayout.getState();
  const tab = activeTab(l);
  // La scheda nuova si carica da sola al primo render (Group.tsx).
  if (tab.file) l.newTab(l.activeGroupId, startLocation(c));
  else void useTabViews.getState().load(tab.id, startLocation(c));
}

/** Ctrl+F: elenco delle connessioni salvate, con nuova, modifica, duplica, elimina e connetti. */
export function ConnectionsDialog({ req }: { req: Extract<DialogRequest, { kind: "connections" }> }) {
  const { t } = useTranslation();
  const list = useConnections((s) => s.list);
  const [selId, setSelId] = useState<string | null>(null);
  // Da un indirizzo scritto a mano: si compila la connessione nuova e, salvata, si apre.
  const [form, setForm] = useState<Draft | null>(req.draft ? blank(req.draft) : null);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // Anteprima dell'importazione da WinSCP: i siti letti e quelli spuntati (per posizione).
  const [imp, setImp] = useState<{ items: ImportCandidate[]; checked: Set<number> } | null>(null);
  // Menu "Importa": una voce per programma da cui importare.
  // Posizione fissa sotto il pulsante: il corpo del dialogo scorre e taglierebbe un menu in `absolute`.
  const [importMenu, setImportMenu] = useState<{ top: number; left: number } | null>(null);
  const importButton = useRef<HTMLButtonElement>(null);
  const close = useUi.getState().closeDialog;
  const sel: ConnectionView | undefined = list.find((c) => c.id === selId) ?? list[0];

  // `hasPassword` cambia anche fuori da qui (connessione con "Ricorda").
  useEffect(() => {
    useConnections.getState().load().catch(console.error);
  }, []);

  const run = (op: () => Promise<unknown>) => {
    setError(null);
    op().catch((e: AppError) => setError(t(`errors.${e?.code ?? "IO"}`)));
  };
  const select = (id: string) => {
    setSelId(id);
    setConfirmDelete(false);
  };
  const connectSel = () => {
    if (!sel) return;
    close();
    openStart(sel);
  };
  const remove = () => {
    if (!sel) return;
    if (!confirmDelete) return setConfirmDelete(true);
    setConfirmDelete(false);
    run(() => useConnections.getState().remove(sel.id));
  };

  // `aria-disabled`: con `disabled` il focus uscirebbe dalla finestra (vedi ConflictDialog).
  const tool = (key: string, enabled: boolean, onClick: () => void, className?: string) => (
    <button className={className} aria-disabled={!enabled || undefined} onClick={() => enabled && onClick()}>{t(key)}</button>
  );

  const onListKey = (e: KeyboardEvent) => {
    const i = list.findIndex((c) => c.id === sel?.id);
    const next = e.key === "ArrowDown" ? i + 1 : e.key === "ArrowUp" ? i - 1 : -1;
    if (next < 0 || next >= list.length) return;
    e.preventDefault();
    select(list[next].id);
  };

  if (imp) {
    const toggle = (i: number) => {
      const checked = new Set(imp.checked);
      if (!checked.delete(i)) checked.add(i);
      setImp({ ...imp, checked });
    };
    const firstFree = imp.items.findIndex((c) => !c.skipped);
    const doImport = () => {
      if (!imp.checked.size) return;
      const chosen = imp.items.filter((_, i) => imp.checked.has(i));
      run(async () => {
        try {
          const views = await importConnections(chosen);
          setImp(null);
          if (views[0]) select(views[0].id);
        } finally {
          await useConnections.getState().load();
        }
      });
    };
    return (
      <Dialog
        key="import"
        title={t("connections.importTitle")}
        onCancel={() => setImp(null)}
        onConfirm={doImport}
        actions={<>
          <button className="primary" aria-disabled={!imp.checked.size || undefined} onClick={doImport}>{t("connections.importAction")}</button>
          <button onClick={() => setImp(null)}>{t("ops.cancel")}</button>
        </>}
      >
        {imp.items.length ? (
          <div className="mc-targets mc-import">
            <div className="mc-target mc-muted"><span />
              <span>{t("connections.name")}</span><span>{t("connections.scheme")}</span><span>{t("connections.host")}</span><span>{t("connections.user")}</span>
            </div>
            {imp.items.map((c, i) => (
              <label key={i} className="mc-target" aria-disabled={!!c.skipped || undefined}>
                <input type="checkbox" checked={imp.checked.has(i)} disabled={!!c.skipped} data-autofocus={i === firstFree || undefined}
                  onChange={() => toggle(i)} />
                <span title={c.name}>{c.name}</span>
                <span>{c.scheme ? t(`connections.schemes.${c.scheme}`) : "-"}</span>
                <span title={c.host}>{c.host}</span>
                <span title={c.user}>{c.user}</span>
                {c.skipped && <em className="mc-muted">{t(`connections.importSkipped.${c.skipped}`, { defaultValue: t("connections.importSkipped.unknown") })}</em>}
              </label>
            ))}
          </div>
        ) : <p className="mc-muted">{t("connections.importNone")}</p>}
        <p className="mc-muted">{t("connections.importHint")}</p>
        {error && <div className="mc-field-error" role="alert">{error}</div>}
      </Dialog>
    );
  }

  if (form) {
    const set = (patch: Partial<Draft>) => setForm({ ...form, ...patch });
    // Lo schema e FTPS implicito portano la porta di default, se non era stata cambiata.
    const setScheme = (scheme: Scheme, ftpsImplicit: boolean) => set({
      scheme, ftpsImplicit,
      port: form.port === defaultPort(form.scheme, form.ftpsImplicit) ? defaultPort(scheme, ftpsImplicit) : form.port,
    });
    const portOk = Number.isInteger(form.port) && form.port > 0 && form.port < 65536;
    const hostOk = validHost(form.host.trim());
    const ok = hostOk && portOk;
    const saved = list.find((c) => c.id === form.id);
    const save = () => {
      if (!ok) return;
      const { password, hasPassword: _, ...conn } = form as Draft & { hasPassword?: boolean };
      const c = { ...conn, host: conn.host.trim(), name: conn.name.trim() || conn.host.trim() };
      run(async () => {
        const view = await useConnections.getState().save(c, password || null, !!password);
        if (req.draft) {
          close();
          openStart(view);
        } else {
          setForm(null);
          select(view.id);
        }
      });
    };
    const cancel = () => (req.draft ? close() : setForm(null));

    return (
      <Dialog
        key="form"
        title={t(form.id ? "connections.editTitle" : "connections.newTitle")}
        onCancel={cancel}
        onConfirm={save}
        actions={<>
          <button className="primary" disabled={!ok} onClick={save}>{t(req.draft ? "connections.saveConnect" : "connections.save")}</button>
          <button onClick={cancel}>{t("ops.cancel")}</button>
        </>}
      >
        <label className="mc-field">
          <span>{t("connections.name")}</span>
          <input type="text" value={form.name} placeholder={form.host} spellCheck={false} onChange={(e) => set({ name: e.target.value })} />
        </label>
        <label className="mc-field">
          <span>{t("connections.scheme")}</span>
          <select value={form.scheme} onChange={(e) => setScheme(e.target.value as Scheme, false)}>
            {SCHEMES.map((s) => <option key={s} value={s}>{t(`connections.schemes.${s}`)}</option>)}
          </select>
        </label>
        {form.scheme === "ftps" && (
          <label className="mc-check">
            <input type="checkbox" checked={form.ftpsImplicit} onChange={(e) => setScheme("ftps", e.target.checked)} />
            {t("connections.ftpsImplicit")}
          </label>
        )}
        <label className="mc-field">
          <span>{t("connections.host")}</span>
          <input type="text" value={form.host} spellCheck={false} data-autofocus aria-invalid={!!form.host.trim() && !hostOk}
            onChange={(e) => set({ host: e.target.value })} />
        </label>
        <label className="mc-field">
          <span>{t("connections.port")}</span>
          <input type="text" inputMode="numeric" value={String(form.port)} aria-invalid={!portOk}
            onChange={(e) => set({ port: Number(e.target.value) })} />
        </label>
        <label className="mc-field">
          <span>{t("connections.user")}</span>
          <input type="text" value={form.user} spellCheck={false} placeholder={t("connections.anonymous")} onChange={(e) => set({ user: e.target.value })} />
        </label>
        <label className="mc-field">
          <span>{t("connections.password")}</span>
          <input type="password" value={form.password} autoComplete="off"
            placeholder={saved?.hasPassword ? t("connections.passwordKept") : ""} onChange={(e) => set({ password: e.target.value })} />
          <span className="mc-field-hint">{t("connections.passwordHint")}</span>
        </label>
        <label className="mc-field">
          <span>{t("connections.startDir")}</span>
          <input type="text" value={form.startDir} spellCheck={false} onChange={(e) => set({ startDir: e.target.value })} />
        </label>
        {error && <div className="mc-field-error" role="alert">{error}</div>}
      </Dialog>
    );
  }

  return (
    <Dialog
      key="list"
      title={t("connections.title")}
      onCancel={close}
      onConfirm={connectSel}
      actions={<>
        <button className="primary" aria-disabled={!sel || undefined} onClick={connectSel}>{t("connections.connect")}</button>
        <button onClick={close}>{t("connections.close")}</button>
      </>}
    >
      {list.length ? (
        <div className="mc-targets" role="listbox" aria-label={t("connections.title")} tabIndex={0} data-autofocus onKeyDown={onListKey}>
          {list.map((c) => (
            <div key={c.id} role="option" aria-selected={c.id === sel?.id} className={`mc-target${c.id === sel?.id ? " sel" : ""}`}
              onClick={() => select(c.id)} onDoubleClick={connectSel}>
              <ConnIcon scheme={c.scheme} />
              <span>{c.name}</span>
              <span className="mc-muted">{c.scheme}://{c.user ? `${c.user}@` : ""}{c.host}:{c.port}</span>
            </div>
          ))}
        </div>
      ) : <p className="mc-muted">{t("connections.empty")}</p>}
      <div className="mc-conn-tools">
        {tool("connections.new", true, () => setForm(blank({})))}
        <div className="mc-conn-import" onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && setImportMenu(null)}>
          <button ref={importButton} aria-haspopup="menu" aria-expanded={!!importMenu} onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setImportMenu(importMenu ? null : { top: r.bottom + 2, left: r.left });
          }}>
            {t("connections.import")} <ChevronDown size={12} aria-hidden />
          </button>
          {importMenu && (
            // Esc chiude il menu, non il dialogo, e il focus torna al pulsante.
            <div className="mc-menu mc-conn-import-menu" role="menu" style={importMenu} onKeyDown={(e) => {
              if (e.key !== "Escape") return;
              e.preventDefault();
              e.stopPropagation();
              setImportMenu(null);
              importButton.current?.focus();
            }}>
              <button role="menuitem" className="mc-menu-item" autoFocus onClick={() => {
                setImportMenu(null);
                run(async () => {
                  const items = await winscpCandidates();
                  setImp({ items, checked: new Set(items.flatMap((c, i) => (c.skipped ? [] : [i]))) });
                });
              }}>
                {t("connections.importWinscp")}
              </button>
            </div>
          )}
        </div>
        {tool("connections.edit", !!sel, () => setForm({ ...sel!, password: "" }))}
        {tool("connections.duplicate", !!sel, () => setForm({ ...sel!, id: "", name: t("connections.copyOf", { name: sel!.name }), password: "" }))}
        {tool(confirmDelete ? "connections.confirmDelete" : "connections.delete", !!sel, remove, confirmDelete ? "danger" : undefined)}
        {tool("connections.forgetPassword", !!sel?.hasPassword, () => run(async () => {
          await forgetPassword(sel!.id);
          await useConnections.getState().load();
        }))}
        {/* FTP e HTTP in chiaro non hanno impronte. */}
        {tool("connections.forgetHost", !!sel && sel.scheme !== "ftp" && sel.scheme !== "http", () => run(() => forgetHost(sel!.id)))}
      </div>
      {error && <div className="mc-field-error" role="alert">{error}</div>}
    </Dialog>
  );
}
