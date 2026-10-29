import { useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { toggleUnpackFolder } from "../../commands/archives";
import { mustChangeDest, resolveDest, startCopyMove } from "../../commands/fileOps";
import type { ConflictPolicy } from "../../ipc/types";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog, useWhat } from "./Dialog";

const POLICIES: ConflictPolicy["type"][] = ["ask", "overwrite", "skip", "overwriteIfNewer", "overwriteIfLarger", "renameNew"];

/** F5/F6: destinazione modificabile, gruppi selezionabili con 1…9, filtro e politica dei conflitti. */
export function CopyMoveDialog({ req }: { req: Extract<DialogRequest, { kind: "copyMove" }> }) {
  const { t } = useTranslation();
  const what = useWhat(req.sources);
  const [dest, setDest] = useState(req.dest);
  // Alt+F9: la sottocartella col nome dell'archivio, attiva all'apertura (la destinazione proposta la contiene).
  const [sub, setSub] = useState(true);
  const toggleSub = (on: boolean) => {
    setSub(on);
    if (req.unpackFolder) setDest(toggleUnpackFolder(dest, req.unpackFolder, on));
  };
  // La cartella di un gruppo come destinazione: con la casella attiva, più la sottocartella (scelta con 1…9 o col clic).
  const inGroup = (path: string) => (req.unpackFolder && sub ? toggleUnpackFolder(path, req.unpackFolder, true) : path);
  const pick = (path: string) => setDest(inGroup(path));
  const [filter, setFilter] = useState("");
  const [policy, setPolicy] = useState(req.policy);
  const close = useUi.getState().closeDialog;
  const resolved = resolveDest(req.from, dest);
  const unchanged = mustChangeDest(req, resolved);
  const ok = resolved !== null && !unchanged;

  const confirm = () => {
    if (!ok) return;
    close();
    startCopyMove(req, resolved, filter, policy).catch(console.error);
  };

  // 1…9 scelgono il gruppo, fuori dai campi di testo (dove le cifre si scrivono).
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
    const target = req.targets.find((g) => String(g.n) === e.key);
    if (target) {
      e.preventDefault();
      pick(target.path);
    }
  };

  return (
    <Dialog
      title={t(req.move ? "ops.moveTitle" : "ops.copyTitle")}
      onCancel={close}
      onConfirm={confirm}
      actions={<>
        <button className="primary" disabled={!ok} onClick={confirm}>{t("ops.ok")}</button>
        <button onClick={close}>{t("ops.cancel")}</button>
      </>}
    >
      <div onKeyDown={onKeyDown}>
        <label className="mc-field">
          <span>{t(req.move ? "ops.moveTo" : "ops.copyTo", { what })}</span>
          <input type="text" value={dest} spellCheck={false} onChange={(e) => setDest(e.target.value)}
            data-autofocus={req.targets.length ? undefined : true} />
        </label>
        {unchanged && <div className="mc-field-hint">{t("ops.sameFolder")}</div>}
        {req.unpackFolder && (
          <label className="mc-check">
            <input type="checkbox" checked={sub} onChange={(e) => toggleSub(e.target.checked)} />
            {t("ops.unpackSubfolder", { name: req.unpackFolder })}
          </label>
        )}
        {req.targets.length > 0 && (
          <div className="mc-targets" role="listbox" aria-label={t("ops.groups")} tabIndex={0} data-autofocus>
            {req.targets.map((g) => (
              <div key={g.groupId} role="option" aria-selected={inGroup(g.path) === dest}
                className={`mc-target${inGroup(g.path) === dest ? " sel" : ""}`} onClick={() => pick(g.path)}>
                <kbd>{g.n <= 9 ? g.n : ""}</kbd>
                <span>{g.path}</span>
              </div>
            ))}
          </div>
        )}
        <label className="mc-field">
          <span>{t("ops.filter")}</span>
          <input type="text" value={filter} placeholder="*.*" spellCheck={false} onChange={(e) => setFilter(e.target.value)} />
        </label>
        <label className="mc-field">
          <span>{t("ops.policy")}</span>
          <select value={policy} onChange={(e) => setPolicy(e.target.value as ConflictPolicy["type"])}>
            {POLICIES.map((p) => <option key={p} value={p}>{t(`ops.policies.${p}`)}</option>)}
          </select>
        </label>
      </div>
    </Dialog>
  );
}
