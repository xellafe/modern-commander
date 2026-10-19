import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { useTranslation } from "react-i18next";
import { folderName } from "../../lib/paths";
import { scrollHandle, type PaneHandle } from "./pane";

/** Immagine servita dall'asset protocol (mai letta in memoria JS): adattata al riquadro, 1:1 al click. */
export function ImagePane({ path, ref }: { path: string; ref?: Ref<PaneHandle> }) {
  const { t } = useTranslation();
  const box = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState(true);
  const [failed, setFailed] = useState(false);

  useImperativeHandle(ref, () => scrollHandle(() => box.current));
  useEffect(() => {
    setFit(true);
    setFailed(false);
  }, [path]);

  if (failed) return <div className="mc-viewer-note">{t("viewer.unsupportedImage")}</div>;
  return (
    <div ref={box} className={`mc-vimage${fit ? " fit" : ""}`}>
      <img src={convertFileSrc(path)} alt={folderName(path)} draggable={false} onClick={() => setFit(!fit)} onError={() => setFailed(true)} />
    </div>
  );
}
