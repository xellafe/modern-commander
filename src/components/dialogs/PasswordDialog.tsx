import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useConnections } from "../../state/connections";
import type { DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/** Password di una connessione: resta nello stato di questo dialogo e passa solo a `done`. */
export function PasswordDialog({ req }: { req: Extract<DialogRequest, { kind: "password" }> }) {
  const { t } = useTranslation();
  const conn = useConnections((s) => s.list.find((c) => c.id === req.id));
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const confirm = () => req.done({ password, remember });
  const cancel = () => req.done(null);

  return (
    <Dialog
      title={t("connections.passwordTitle")}
      onCancel={cancel}
      onConfirm={confirm}
      actions={<>
        <button className="primary" onClick={confirm}>{t("ops.ok")}</button>
        <button onClick={cancel}>{t("ops.cancel")}</button>
      </>}
    >
      {req.failed && <div className="mc-field-error" role="alert">{t("errors.AUTH_FAILED")}</div>}
      <p>{t("connections.passwordFor", { name: conn?.name ?? req.id })}</p>
      {conn && <div className="mc-path">{conn.user ? `${conn.user}@` : ""}{conn.host}:{conn.port}</div>}
      <label className="mc-field">
        <span>{t("connections.password")}</span>
        <input type="password" value={password} autoComplete="off" data-autofocus onChange={(e) => setPassword(e.target.value)} />
      </label>
      <label className="mc-check">
        <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
        {t("connections.remember")}
      </label>
    </Dialog>
  );
}
