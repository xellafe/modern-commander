import { useTranslation } from "react-i18next";
import { deleteSources } from "../../commands/fileOps";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog, useWhat } from "./Dialog";

/** F8 (nel Cestino) e Shift+F8 (definitivamente): conferma con cosa e quanti elementi. */
export function DeleteConfirm({ req }: { req: Extract<DialogRequest, { kind: "delete" }> }) {
  const { t } = useTranslation();
  const what = useWhat(req.sources);
  const close = useUi.getState().closeDialog;
  const confirm = () => {
    close();
    deleteSources(req.sources, req.permanent).catch(console.error);
  };

  return (
    <Dialog
      title={t(req.permanent ? "ops.deletePermanentTitle" : "ops.deleteTitle")}
      onCancel={close}
      actions={<>
        <button className={req.permanent ? "danger" : "primary"} onClick={confirm} data-autofocus>{t("ops.delete")}</button>
        <button onClick={close}>{t("ops.cancel")}</button>
      </>}
    >
      <p>{t(req.permanent ? "ops.deletePermanentText" : "ops.deleteText", { what })}</p>
    </Dialog>
  );
}
