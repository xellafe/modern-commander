import { useTranslation } from "react-i18next";
import { folderName } from "../../lib/paths";
import { retryOptions, useEditors, type SaveOptions } from "../../state/editors";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/**
 * Salvataggio che perderebbe dati: file cambiato da un altro programma (sovrascrivi o annulla), caratteri che la
 * codifica non rappresenta (salva in UTF-8 o annulla), server senza scrittura sicura (salva lo stesso o annulla).
 * Il pulsante sicuro ha il focus. Su un server con date poco affidabili ogni salvataggio si conferma qui (focus su Annulla).
 */
export function SaveErrorDialog({ req }: { req: Extract<DialogRequest, { kind: "saveError" }> }) {
  const { t } = useTranslation();
  const doc = useEditors((s) => s.docs[req.tabId]);
  const close = useUi.getState().closeDialog;
  const saveWith = (opts: SaveOptions) => {
    close();
    void useEditors.getState().save(req.tabId, opts);
  };
  const retry = () => saveWith(retryOptions(req.opts, req.code));
  const name = folderName(doc?.path ?? "");
  const risky = req.code === "CHANGED_ON_DISK" || req.code === "UNSAFE_WRITE";
  // Anche la conferma per le date poco affidabili parte da Annulla: un Invio distratto non salva.
  const cancelFirst = risky || req.code === "UNRELIABLE_MTIME";
  const [title, text, action] = {
    CHANGED_ON_DISK: ["editor.saveErrorTitle", t("editor.changedOnDisk", { name }), "editor.overwrite"],
    UNMAPPABLE_CHARS: ["editor.saveErrorTitle", t("editor.unmappable", { name, encoding: doc?.encoding }), "editor.saveUtf8"],
    UNSAFE_WRITE: ["ops.unsafeWriteTitle", t("ops.unsafeWrite"), "ops.saveAnyway"],
    UNRELIABLE_MTIME: ["editor.unreliableMtimeTitle", t("editor.unreliableMtime", { name }), "editor.save"],
  }[req.code];

  return (
    <Dialog
      title={t(title)}
      onCancel={close}
      actions={<>
        {risky
          ? <button className="danger" onClick={retry}>{t(action)}</button>
          : <button className="primary" onClick={retry} data-autofocus={!cancelFirst || undefined}>{t(action)}</button>}
        <button onClick={close} data-autofocus={cancelFirst || undefined}>{t("ops.cancel")}</button>
      </>}
    >
      <p>{text}</p>
    </Dialog>
  );
}
