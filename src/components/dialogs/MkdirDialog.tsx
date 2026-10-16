import { useState } from "react";
import { useTranslation } from "react-i18next";
import { createFolder, mkdirTarget } from "../../commands/fileOps";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/** F7: nome relativo alla cartella corrente (`\` crea le intermedie) o percorso assoluto. */
export function MkdirDialog({ req }: { req: Extract<DialogRequest, { kind: "mkdir" }> }) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const close = useUi.getState().closeDialog;
  const target = mkdirTarget(req.path, name);
  const error = name.trim() && "error" in target ? target.error : null;

  const confirm = () => {
    if ("error" in target) return;
    close();
    createFolder(req.tabId, req.path, name).catch(console.error);
  };

  return (
    <Dialog
      title={t("ops.mkdirTitle")}
      onCancel={close}
      onConfirm={confirm}
      actions={<>
        <button className="primary" disabled={"error" in target} onClick={confirm}>{t("ops.ok")}</button>
        <button onClick={close}>{t("ops.cancel")}</button>
      </>}
    >
      <label className="mc-field">
        <span>{t("ops.mkdirLabel", { path: req.path })}</span>
        <input type="text" value={name} spellCheck={false} aria-invalid={!!error} onChange={(e) => setName(e.target.value)} />
      </label>
      {error && <div className="mc-field-error" role="alert">{t(error)}</div>}
    </Dialog>
  );
}
