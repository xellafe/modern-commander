import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ErrorAction, Prompt } from "../../ipc/types";
import { Dialog } from "./Dialog";
import { useAnswer } from "./ConflictDialog";

type ErrorPrompt = Extract<Prompt, { type: "error" }>;

/** Errore su un elemento: riprova, salta, annulla; come amministratore (Task 17) ed eliminazione definitiva se offerte. */
export function ErrorDialog({ prompt }: { prompt: ErrorPrompt }) {
  const { t } = useTranslation();
  const { busy, error, send } = useAnswer(prompt);
  const [all, setAll] = useState(false);
  const act = (type: ErrorAction["type"]) => void send({ type: "error", action: { type }, applyToAll: all });
  const button = (type: ErrorAction["type"], key: string, autofocus = false) => (
    <button disabled={busy} onClick={() => act(type)} data-autofocus={autofocus || undefined}>{t(key)}</button>
  );
  const e = prompt.error;

  return (
    <Dialog
      title={t("ops.errorTitle")}
      onCancel={() => act("cancel")}
      actions={<>
        {button("retry", "ops.retry", true)}
        {prompt.canElevate && button("retryElevated", "ops.retryElevated")}
        {button("skip", "ops.skip")}
        {prompt.canDeletePermanently && button("deletePermanent", "ops.deletePermanentNow")}
        {button("cancel", "ops.cancel")}
      </>}
    >
      {/* `detail` è tecnico (testo di sistema o codice interno): solo come suggerimento. */}
      <p title={e.detail ?? undefined}>{t(`errors.${e.code}`)}</p>
      {e.path && <div className="mc-path">{e.path}</div>}
      <label className="mc-check">
        <input type="checkbox" checked={all} onChange={(ev) => setAll(ev.target.checked)} />
        {t("ops.applyToAllErrors")}
      </label>
      {error && <div className="mc-field-error" role="alert">{error}</div>}
    </Dialog>
  );
}
