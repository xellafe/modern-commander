import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openPath } from "../../ipc/api";

/** Video o audio col lettore della WebView; il file arriva dall'asset protocol con richieste Range. */
export function MediaPane({ path, kind, autoPlay }: { path: string; kind: "video" | "audio"; autoPlay: boolean }) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [path]);

  if (failed) {
    return (
      <div className="mc-viewer-note">
        {t("viewer.unsupportedMedia")}
        {/* Senza una scheda a cui riferire, un errore dell'apertura esterna non ha dove comparire. */}
        <button onClick={() => openPath(path).catch(() => {})}>{t("viewer.openExternal")}</button>
      </div>
    );
  }
  const props = { src: convertFileSrc(path), controls: true, autoPlay, onError: () => setFailed(true) };
  return (
    <div className="mc-vmedia">
      {kind === "video" ? <video {...props} /> : <audio {...props} />}
    </div>
  );
}
