import { useTranslation } from "react-i18next";
import { useConnections } from "../../state/connections";
import type { DialogRequest } from "../../state/ui";
import { Dialog } from "./Dialog";

/** Impronta sconosciuta (chiave SSH o certificato TLS): Accetta la ricorda per host e porta. Mai "accetta tutto". */
export function TrustDialog({ req }: { req: Extract<DialogRequest, { kind: "trust" }> }) {
  const { t } = useTranslation();
  const conn = useConnections((s) => s.list.find((c) => c.id === req.id));
  const kind = req.tls ? "cert" : "hostKey";

  return (
    <Dialog
      title={t(`connections.${kind}Title`)}
      onCancel={() => req.done(false)}
      actions={<>
        <button onClick={() => req.done(true)}>{t("connections.accept")}</button>
        <button className="primary" data-autofocus onClick={() => req.done(false)}>{t("ops.cancel")}</button>
      </>}
    >
      <p>{t(`connections.${kind}Unknown`, { name: conn?.name ?? req.id })}</p>
      {conn && <div className="mc-path">{conn.host}:{conn.port}</div>}
      <div className="mc-field">
        <span>{t("connections.fingerprint")}</span>
        <div className="mc-path">{req.fingerprint}</div>
      </div>
      <p className="mc-muted">{t(`connections.${kind}Hint`)}</p>
    </Dialog>
  );
}
