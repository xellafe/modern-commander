import { Copy } from "lucide-react";
import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { HashAlgo } from "../../ipc/types";
import { formatAttrs, formatDate, formatSize } from "../../lib/format";
import { loadMonaco, splitForColorize, worthColorizing } from "../../lib/monaco";
import { folderName, joinPath } from "../../lib/paths";
import { PREVIEW_BYTES, useInspector, type Preview } from "../../state/inspector";
import { useLayout } from "../../state/layout";
import { useTabViews } from "../../state/tabViews";
import { useUi } from "../../state/ui";
import { Splitter } from "../shell/Splitter";
import { ImagePane } from "../viewer/ImagePane";
import { MediaPane } from "../viewer/MediaPane";
import { PdfPane } from "../viewer/PdfPane";
import "../viewer/viewer.css";
import "./inspector.css";

const HASHES: [HashAlgo, string][] = [["md5", "MD5"], ["sha1", "SHA-1"], ["sha256", "SHA-256"]];

/**
 * Segue cursore, scheda e gruppo attivi e passa il bersaglio a `useInspector`. Sta in un componente suo,
 * senza output: a ogni passo del cursore si rifà solo questo, non il pannello.
 */
function TargetFeed() {
  const tabId = useLayout((s) => s.groups.find((g) => g.id === s.activeGroupId)?.activeTabId);
  const tab = useLayout((s) => (tabId ? s.tabs[tabId] : undefined));
  const row = useTabViews((s) => {
    const v = tabId ? s.views[tabId] : undefined;
    return v?.rows[v.cursor];
  });
  const drive = row?.drive;
  // `..`, schede di file e cartelle vuote non hanno un bersaglio.
  const path = !tab || tab.file || !row || row.isParent ? null : drive ? row.name : joinPath(tab.path, row.name);
  const isDir = !!row?.isDir;

  // Il ritmo dei cambi lo regola il debounce di `setTarget`; `drive` conta per la radice (il polling lo rinnova di continuo).
  useEffect(() => {
    useInspector.getState().setTarget(path === null ? null : { path, isDir, drive });
  }, [path, isDir, drive?.root]); // eslint-disable-line react-hooks/exhaustive-deps
  // Nascosto: si annulla tutto ciò che è in corso.
  useEffect(() => () => useInspector.getState().setTarget(null), []);
  return null;
}

/**
 * Testo semplice subito; le prime righe colorate da Monaco appena pronto, il resto resta semplice (`colorize` gira
 * sul thread della UI). L'HTML di `colorize` nasce da testo con `<`, `>` e `&` già sostituiti.
 */
function TextPreview({ text, language }: { text: string; language: string | null }) {
  const [html, setHtml] = useState<string | null>(null);
  const [head, tail] = useMemo(() => splitForColorize(text), [text]);
  useEffect(() => {
    setHtml(null);
    if (!worthColorizing(head, language)) return; // resta testo semplice
    let live = true;
    loadMonaco()
      .then((m) => m.editor.colorize(head, language, { tabSize: 4 }))
      .then((h) => live && setHtml(h))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [head, language]);
  return html === null ? (
    <pre className="mc-inspector-text">{text}</pre>
  ) : (
    <pre className="mc-inspector-text"><span dangerouslySetInnerHTML={{ __html: html }} />{tail}</pre>
  );
}

/** I link del Markdown non navigano mai (click, tasto centrale, menu contestuale "apri in una nuova finestra"). */
const blockLinks = (e: MouseEvent) => {
  if ((e.target as Element).closest("a")) e.preventDefault();
};

function PreviewView({ preview }: { preview: Preview | null }) {
  const { t, i18n } = useTranslation();
  const lng = i18n.language;
  if (!preview || preview.kind === "loading") return <div className="mc-viewer-note">{t("inspector.loading")}</div>;
  const truncated = (
    <div className="mc-viewer-note">{t("inspector.truncated", { size: formatSize(PREVIEW_BYTES, lng) })}</div>
  );
  switch (preview.kind) {
    case "image": return <ImagePane key={preview.path} path={preview.path} />;
    case "pdf": return <PdfPane key={preview.path} path={preview.path} />;
    case "video":
    case "audio": return <MediaPane key={preview.path} path={preview.path} kind={preview.kind} autoPlay={false} />;
    case "text":
      return (
        <>
          <TextPreview text={preview.text} language={preview.language} />
          {preview.truncated && truncated}
        </>
      );
    case "markdown":
      return (
        <>
          <div className="mc-inspector-md" onClick={blockLinks} onAuxClick={blockLinks} onContextMenu={blockLinks} dangerouslySetInnerHTML={{ __html: preview.html }} />
          {preview.truncated && truncated}
        </>
      );
    case "folder":
      return (
        <div className="mc-inspector-facts">
          {preview.size ? (
            <>
              <div>{formatSize(preview.size.bytes, lng)}</div>
              <div>{t("inspector.folderContent", { files: preview.size.files, dirs: preview.size.dirs })}</div>
              {preview.size.skipped > 0 && <div>{t("inspector.folderSkipped", { n: preview.size.skipped })}</div>}
            </>
          ) : (
            <div>{t("inspector.folderCalculating")}</div>
          )}
        </div>
      );
    case "drive": {
      const d = preview.drive;
      return (
        <div className="mc-inspector-facts">
          <div>{d.label || d.root}</div>
          {d.ready && <div>{t("inspector.driveFree", { free: formatSize(d.free, lng), total: formatSize(d.total, lng) })}</div>}
        </div>
      );
    }
    case "archive": {
      const { entries, total, truncated: more } = preview.listing;
      return (
        <>
          <div className="mc-viewer-note">
            {more && total === null ? t("inspector.archiveFirst", { n: entries.length }) : t("inspector.archiveTotal", { n: total ?? entries.length })}
          </div>
          <ul className="mc-inspector-archive">
            {entries.map((e) => (
              <li key={e.path} title={e.path}>
                <span>{e.path}{e.isDir ? "\\" : ""}</span>
                {!e.isDir && <span>{formatSize(e.size, lng)}</span>}
              </li>
            ))}
          </ul>
        </>
      );
    }
    case "archiveOnDemand":
      return (
        <div className="mc-inspector-facts">
          <button onMouseDown={(e) => e.preventDefault()} onClick={() => useInspector.getState().requestArchive()}>
            {t("inspector.showArchiveContents")}
          </button>
        </div>
      );
    case "binary": return <div className="mc-viewer-note">{t("inspector.binary")}</div>;
    case "error": return <div className="mc-viewer-note" role="alert">{t(`errors.${preview.code}`)}</div>;
  }
}

function Hashes() {
  const { t } = useTranslation();
  const hashes = useInspector((s) => s.hashes);
  const requestHash = useInspector((s) => s.requestHash);
  return (
    <div className="mc-inspector-hashes">
      {HASHES.map(([algo, label]) => {
        const h = hashes[algo];
        return (
          <div key={algo}>
            <button disabled={h !== undefined} onMouseDown={(e) => e.preventDefault()} onClick={() => requestHash(algo)}>
              {label}
            </button>
            {h === "pending" && <span className="mc-inspector-pending">{t("inspector.hashing")}</span>}
            {h && h !== "pending" && (
              <>
                <code>{h}</code>
                <button
                  title={t("inspector.copy")}
                  aria-label={t("inspector.copy")}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => void navigator.clipboard.writeText(h).catch(() => {})}
                >
                  <Copy size={12} />
                </button>
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}

function Body() {
  const { t, i18n } = useTranslation();
  const lng = i18n.language;
  const target = useInspector((s) => s.target);
  const info = useInspector((s) => s.info);
  const preview = useInspector((s) => s.preview);
  if (!target) return <div className="mc-viewer-note">{t("inspector.empty")}</div>;
  const isFile = !target.isDir && !target.drive;
  return (
    <>
      <div className="mc-inspector-name" title={target.path}>{folderName(target.path)}</div>
      <div className="mc-inspector-preview"><PreviewView preview={preview} /></div>
      {info && (
        <dl className="mc-inspector-meta">
          {isFile && (
            <>
              <dt>{t("inspector.size")}</dt>
              <dd>{formatSize(info.size, lng)} ({t("inspector.sizeExact", { bytes: info.size.toLocaleString(lng) })})</dd>
            </>
          )}
          <dt>{t("inspector.created")}</dt>
          <dd>{formatDate(info.createdMs, lng)}</dd>
          <dt>{t("inspector.modified")}</dt>
          <dd>{formatDate(info.modifiedMs, lng)}</dd>
          <dt>{t("inspector.accessed")}</dt>
          <dd>{formatDate(info.accessedMs, lng)}</dd>
          <dt>{t("inspector.attributes")}</dt>
          <dd>{formatAttrs(info.attributes)}</dd>
        </dl>
      )}
      {isFile && info && <Hashes />}
    </>
  );
}

/** Pannello a destra (Ctrl+Q). Montato solo se visibile: nascondendolo `TargetFeed` annulla il lavoro in corso. */
export function Inspector() {
  const { t } = useTranslation();
  const width = useUi((s) => s.inspectorWidth);
  const setWidth = useUi((s) => s.setInspectorWidth);
  return (
    <aside className="mc-inspector" style={{ width }} aria-label={t("inspector.title")}>
      <Splitter axis="x" size={width} onResize={setWidth} invert />
      <TargetFeed />
      <Body />
    </aside>
  );
}
