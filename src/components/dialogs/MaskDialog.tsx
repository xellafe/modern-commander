import { useState } from "react";
import { useTranslation } from "react-i18next";
import { applyMask } from "../../commands/fileOps";
import { useUi, type DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/** Num+ marca, Num- smarca gli elementi che corrispondono alla maschera TC. */
export function MaskDialog({ req }: { req: Extract<DialogRequest, { kind: "mask" }> }) {
  const { t } = useTranslation();
  const [mask, setMask] = useState("*.*");
  const close = useUi.getState().closeDialog;
  const confirm = () => {
    close();
    if (mask.trim()) applyMask(req.tabId, mask.trim(), req.select);
  };

  return (
    <Dialog
      title={t(req.select ? "ops.selectTitle" : "ops.unselectTitle")}
      onCancel={close}
      onConfirm={confirm}
      actions={<>
        <button className="primary" onClick={confirm}>{t("ops.ok")}</button>
        <button onClick={close}>{t("ops.cancel")}</button>
      </>}
    >
      <label className="mc-field">
        <span>{t("ops.mask")}</span>
        <input type="text" value={mask} spellCheck={false} onChange={(e) => setMask(e.target.value)} />
      </label>
    </Dialog>
  );
}
