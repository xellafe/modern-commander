import { useState } from "react";
import { useTranslation } from "react-i18next";
import { packSubmit, zipName } from "../../commands/archives";
import { nameError } from "../../commands/fileOps";
import { isArchive, normalizePath, sourceCaps } from "../../lib/paths";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog, useWhat } from "./Dialog";

/** Alt+F5: nome e cartella dello ZIP. Se il file esiste già, OK diventa "Sovrascrivi"; cambiare il nome o la cartella toglie l'avviso. */
export function PackDialog({ req }: { req: Extract<DialogRequest, { kind: "pack" }> }) {
  const { t } = useTranslation();
  const what = useWhat(req.sources);
  const [name, setName] = useState(req.name);
  const [destDir, setDestDir] = useState(req.destDir);
  const [exists, setExists] = useState(false);
  const close = useUi.getState().closeDialog;
  const dir = normalizePath(destDir);
  const ok = !nameError(zipName(name), dir) && !!dir && sourceCaps(dir).write && !isArchive(dir);

  const confirm = async () => {
    if (!ok) return;
    try {
      if (await packSubmit(req, name, dir, exists)) close();
      else setExists(true);
    } catch (e) {
      console.error(e);
      close();
    }
  };
  const edit = (set: (v: string) => void) => (e: { target: { value: string } }) => {
    set(e.target.value);
    setExists(false);
  };

  return (
    <Dialog
      title={t("ops.packTitle")}
      onCancel={close}
      onConfirm={() => void confirm()}
      actions={<>
        <button className="primary" disabled={!ok} onClick={() => void confirm()}>{t(exists ? "ops.overwrite" : "ops.ok")}</button>
        <button onClick={close}>{t("ops.cancel")}</button>
      </>}
    >
      <label className="mc-field">
        <span>{t("ops.packName", { what })}</span>
        <input type="text" value={name} spellCheck={false} data-autofocus onChange={edit(setName)} />
      </label>
      <label className="mc-field">
        <span>{t("ops.packDest")}</span>
        <input type="text" value={destDir} spellCheck={false} onChange={edit(setDestDir)} />
      </label>
      {exists && <div className="mc-field-error" role="alert">{t("ops.packExists", { name: zipName(name) })}</div>}
    </Dialog>
  );
}
