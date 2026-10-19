import { useTranslation } from "react-i18next";
import { closeTabNow } from "../../commands/navigation";
import { folderName } from "../../lib/paths";
import { useEditors } from "../../state/editors";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/** Chiusura di un editor modificato: salvare e chiudere, chiudere perdendo le modifiche, oppure restare. */
export function UnsavedDialog({ req }: { req: Extract<DialogRequest, { kind: "unsaved" }> }) {
  const { t } = useTranslation();
  const name = useEditors((s) => folderName(s.docs[req.tabId]?.path ?? ""));
  const close = useUi.getState().closeDialog;
  const save = async () => {
    close(); // prima di salvare: un errore di salvataggio apre la sua finestra
    await useEditors.getState().save(req.tabId);
    // Se il salvataggio è fallito (finestra o avviso) la scheda resta aperta, con le sue modifiche.
    if (useEditors.getState().docs[req.tabId]?.dirty === false) closeTabNow(req.tabId);
  };
  const discard = () => {
    close();
    closeTabNow(req.tabId);
  };

  return (
    <Dialog
      title={t("editor.unsavedTitle")}
      onCancel={close}
      actions={<>
        <button className="primary" onClick={() => void save()} data-autofocus>{t("editor.save")}</button>
        <button className="danger" onClick={discard}>{t("editor.discard")}</button>
        <button onClick={close}>{t("ops.cancel")}</button>
      </>}
    >
      <p>{t("editor.unsavedText", { name })}</p>
    </Dialog>
  );
}
