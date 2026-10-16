import { useTranslation } from "react-i18next";
import { exitCancellingJobs } from "../../commands/navigation";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/** Chiusura con operazioni non finite: annullarle e uscire, oppure restare. */
export function ExitConfirm({ req }: { req: Extract<DialogRequest, { kind: "exit" }> }) {
  const { t } = useTranslation();
  const close = useUi.getState().closeDialog;
  const confirm = () => {
    close();
    exitCancellingJobs().catch(console.error);
  };

  return (
    <Dialog
      title={t("ops.exitTitle")}
      onCancel={close}
      actions={<>
        <button className="danger" onClick={confirm}>{t("ops.exitConfirm")}</button>
        <button onClick={close} data-autofocus>{t("ops.cancel")}</button>
      </>}
    >
      <p>{t("ops.exitText", { count: req.count })}</p>
    </Dialog>
  );
}
