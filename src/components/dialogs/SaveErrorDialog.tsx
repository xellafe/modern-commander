import { useTranslation } from "react-i18next";
import { folderName } from "../../lib/paths";
import { retryOptions, useEditors, type SaveOptions } from "../../state/editors";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/**
 * Salvataggio che perderebbe dati: file cambiato da un altro programma (sovrascrivi o annulla) oppure caratteri
 * che la codifica non rappresenta (salva in UTF-8 o annulla). Il pulsante sicuro ha il focus.
 */
export function SaveErrorDialog({ req }: { req: Extract<DialogRequest, { kind: "saveError" }> }) {
  const { t } = useTranslation();
  const doc = useEditors((s) => s.docs[req.tabId]);
  const close = useUi.getState().closeDialog;
  const saveWith = (opts: SaveOptions) => {
    close();
    void useEditors.getState().save(req.tabId, opts);
  };
  const changed = req.code === "CHANGED_ON_DISK";
  const name = folderName(doc?.path ?? "");

  return (
    <Dialog
      title={t("editor.saveErrorTitle")}
      onCancel={close}
      actions={<>
        {changed
          ? <button className="danger" onClick={() => saveWith(retryOptions(req.opts, req.code))}>{t("editor.overwrite")}</button>
          : <button className="primary" onClick={() => saveWith(retryOptions(req.opts, req.code))} data-autofocus>{t("editor.saveUtf8")}</button>}
        <button onClick={close} data-autofocus={changed || undefined}>{t("ops.cancel")}</button>
      </>}
    >
      <p>{changed ? t("editor.changedOnDisk", { name }) : t("editor.unmappable", { name, encoding: doc?.encoding })}</p>
    </Dialog>
  );
}
