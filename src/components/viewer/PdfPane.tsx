import { convertFileSrc } from "@tauri-apps/api/core";
import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { useTranslation } from "react-i18next";
import { scrollHandle, type PaneHandle } from "./pane";

/** Le pagine entrano nel disegno un po' prima di comparire, così scorrendo non si vedono vuote. */
const PRELOAD_MARGIN = "400px 0px";

/** Una pagina: segnaposto della sua altezza, canvas disegnato solo mentre è (quasi) in vista. */
function PdfPage({ doc, n, width, ratio, root }: { doc: PDFDocumentProxy; n: number; width: number; ratio: number; root: HTMLElement }) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [visible, setVisible] = useState(false);
  const [own, setOwn] = useState<number | null>(null); // proporzioni reali, note dopo il primo disegno

  useEffect(() => {
    const io = new IntersectionObserver(([e]) => setVisible(e.isIntersecting), { root, rootMargin: PRELOAD_MARGIN });
    io.observe(wrap.current!);
    return () => io.disconnect();
  }, [root]);

  useEffect(() => {
    if (!visible) return;
    const cv = canvas.current!;
    let live = true;
    let task: RenderTask | undefined;
    (async () => {
      const page = await doc.getPage(n);
      if (!live) return;
      const base = page.getViewport({ scale: 1 });
      const viewport = page.getViewport({ scale: (width / base.width) * (window.devicePixelRatio || 1) });
      cv.width = viewport.width;
      cv.height = viewport.height;
      setOwn(base.height / base.width);
      task = page.render({ canvas: cv, viewport });
      await task.promise;
    })().catch(() => {}); // l'annullamento al cleanup rifiuta la promessa; una pagina guasta resta bianca
    return () => {
      live = false;
      task?.cancel();
      cv.width = cv.height = 0; // fuori vista la bitmap si libera: con 300 pagine non resterebbero in memoria
    };
  }, [doc, n, width, visible]);

  return (
    <div ref={wrap} className="mc-vpdf-page" style={{ width, height: Math.round(width * (own ?? ratio)) }}>
      <canvas ref={canvas} />
    </div>
  );
}

/** PDF servito dall'asset protocol (pdf.js legge a intervalli solo le pagine richieste, `disableAutoFetch`; mai il file intero); pagine scalate alla larghezza, senza livello di testo. */
export function PdfPane({ path, ref }: { path: string; ref?: Ref<PaneHandle> }) {
  const { t } = useTranslation();
  const box = useRef<HTMLDivElement>(null);
  const [loaded, setLoaded] = useState<{ doc: PDFDocumentProxy; ratio: number } | null>(null);
  const [failed, setFailed] = useState(false);
  const [width, setWidth] = useState(0);

  useImperativeHandle(ref, () => scrollHandle(() => box.current));

  useEffect(() => {
    const el = box.current!;
    const ro = new ResizeObserver(([e]) => setWidth(Math.floor(e.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // pdf.js e il suo worker sono caricati solo qui, in chunk separati dal principale.
  useEffect(() => {
    let live = true;
    let task: { destroy(): Promise<void> } | undefined;
    setLoaded(null);
    setFailed(false);
    (async () => {
      const [pdfjs, worker] = await Promise.all([import("pdfjs-dist"), import("pdfjs-dist/build/pdf.worker.min.mjs?url")]);
      if (!live) return; // chiuso mentre i chunk si caricavano: `task` è ancora vuoto, non si deve creare nulla
      pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
      // Niente `isEvalSupported`: in questa versione di pdf.js l'opzione non esiste più (non c'è eval da disattivare).
      // `disableAutoFetch`: legge a intervalli solo le pagine richieste, mai il file intero in background.
      const loading = pdfjs.getDocument({ url: convertFileSrc(path), disableAutoFetch: true });
      task = loading;
      const doc = await loading.promise;
      const { width: w, height: h } = (await doc.getPage(1)).getViewport({ scale: 1 }); // stima per i segnaposto
      if (!live) return void loading.destroy(); // cleanup passato durante la lettura
      setLoaded({ doc, ratio: h / w });
    })().catch(() => live && setFailed(true));
    return () => {
      live = false;
      void task?.destroy(); // chiude anche il documento e il worker
    };
  }, [path]);

  return (
    <div ref={box} className="mc-vpdf">
      {failed && <div className="mc-viewer-note">{t("viewer.unsupportedPdf")}</div>}
      {loaded && width > 0 &&
        Array.from({ length: loaded.doc.numPages }, (_, i) => (
          <PdfPage key={i} doc={loaded.doc} n={i + 1} width={width} ratio={loaded.ratio} root={box.current!} />
        ))}
    </div>
  );
}
