import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { useTranslation } from "react-i18next";
import { thumbnail } from "../../ipc/api";
import { folderName } from "../../lib/paths";
import { scrollHandle, type PaneHandle } from "./pane";

/** Immagine servita dall'asset protocol (mai letta in memoria JS): adattata al riquadro, 1:1 al click. */
export function ImagePane({ path, ref }: { path: string; ref?: Ref<PaneHandle> }) {
  const box = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState(true);
  const [failed, setFailed] = useState(false);

  useImperativeHandle(ref, () => scrollHandle(() => box.current));
  useEffect(() => {
    setFit(true);
    setFailed(false);
  }, [path]);

  if (failed) return <ShellPreview path={path} />;
  return (
    <div ref={box} className={`mc-vimage${fit ? " fit" : ""}`}>
      <img src={convertFileSrc(path)} alt={folderName(path)} draggable={false} onClick={() => setFit(!fit)} onError={() => setFailed(true)} />
    </div>
  );
}

/**
 * Formati che la WebView non decodifica (TIFF, HEIC, PSD, ...): se la shell ha un codec installato ne dà
 * l'anteprima come in Explorer, a 1024 px al massimo; altrimenti resta la nota di formato non supportato.
 */
function ShellPreview({ path }: { path: string }) {
  const { t } = useTranslation();
  const canvas = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<"loading" | "ok" | "failed">("loading");
  useEffect(() => {
    let live = true;
    setState("loading");
    thumbnail(path, 1024).then(
      ({ width, height, data }) => {
        const c = canvas.current;
        if (!live || !c) return;
        c.width = width;
        c.height = height;
        c.getContext("2d")!.putImageData(new ImageData(data as Uint8ClampedArray<ArrayBuffer>, width, height), 0, 0);
        setState("ok");
      },
      () => live && setState("failed"),
    );
    return () => {
      live = false;
    };
  }, [path]);

  if (state === "failed") return <div className="mc-viewer-note">{t("viewer.unsupportedImage")}</div>;
  return (
    <>
      {state === "ok" && <div className="mc-viewer-note">{t("viewer.shellPreview")}</div>}
      <div className="mc-vimage fit" style={state === "ok" ? undefined : { display: "none" }}>
        <canvas ref={canvas} aria-label={folderName(path)} />
      </div>
    </>
  );
}
