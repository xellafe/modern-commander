import type { editor } from "monaco-editor";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { chordFromEvent } from "../../commands/keys";
import { closeTab } from "../../commands/navigation";
import { executeCommand } from "../../commands/registry";
import type { AppError } from "../../ipc/types";
import { formatSize } from "../../lib/format";
import { loadMonaco } from "../../lib/monaco";
import { folderName } from "../../lib/paths";
import { EDITOR_MAX_BYTES, editorModel, savedViewState, saveViewState, useEditors } from "../../state/editors";
import { useLayout } from "../../state/layout";
import { useTabViews } from "../../state/tabViews";
import "./editor.css";

const toAppError = (e: unknown): AppError =>
  e && typeof e === "object" && "code" in e ? (e as AppError) : { code: "IO", path: null, detail: null };

const inActiveGroup = (tabId: string) => {
  const l = useLayout.getState();
  return l.groups.find((g) => g.tabIds.includes(tabId))?.id === l.activeGroupId;
};

/**
 * F4: editor Monaco di una scheda di file. L'istanza nasce al montaggio e muore allo smontaggio (cambio
 * scheda); il modello, la posizione e le modifiche stanno in `useEditors` e tornano alla riapertura.
 */
export function Editor({ tabId, path }: { tabId: string; path: string }) {
  const { t, i18n } = useTranslation();
  const name = folderName(path);
  const doc = useEditors((s) => s.docs[tabId]);
  const active = useLayout((s) => s.groups.find((g) => g.tabIds.includes(tabId))?.id === s.activeGroupId);
  const notice = useTabViews((s) => (s.notice?.tabId === tabId ? s.notice : null));
  const [error, setError] = useState<AppError | null>(null);
  const host = useRef<HTMLDivElement>(null);
  const instance = useRef<editor.IStandaloneCodeEditor | null>(null);
  const forceReadOnly = useLayout((s) => !!s.tabs[tabId].file?.readOnly);
  const loaded = !!doc;
  const readOnly = doc?.readOnly ?? false;

  useEffect(() => {
    useEditors.getState().open(tabId, path, forceReadOnly).catch((e) => setError(toAppError(e)));
  }, [tabId, path]);

  useEffect(() => {
    if (!loaded) return;
    let live = true;
    let ed: editor.IStandaloneCodeEditor | undefined;
    const actions: { dispose(): void }[] = [];
    void loadMonaco().then((monaco) => {
      const model = editorModel(tabId);
      if (!live || !model) return; // smontato o chiuso mentre Monaco si caricava
      ed = monaco.editor.create(host.current!, {
        model,
        readOnly,
        automaticLayout: true,
        minimap: { enabled: true },
        scrollBeyondLastLine: false,
        fontFamily: getComputedStyle(host.current!).getPropertyValue("--mc-font-mono"),
        fontSize: 12,
        fontLigatures: false,
      });
      instance.current = ed;
      const view = savedViewState(tabId);
      if (view) ed.restoreViewState(view);
      // Il focus è nel campo di testo di Monaco: useKeyboard lascia passare solo i comandi globali, quindi
      // Ctrl+S, Ctrl+W e Ctrl+Tab vanno agganciati qui. `addAction` (non `addCommand`, che registra un tasto globale
      // senza contesto e non si può togliere) vale solo per questa istanza e si toglie allo smontaggio.
      actions.push(
        ed.addAction({
          id: "mc.save",
          label: i18n.t("editor.save"),
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
          run: () => void useEditors.getState().save(tabId),
        }),
        ed.addAction({
          id: "mc.closeTab",
          label: i18n.t("groups.closeTab"),
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW],
          run: () => closeTab(tabId),
        }),
        ed.addAction({
          id: "mc.nextTab",
          label: i18n.t("commands.cm_SwitchToNextTab"),
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Tab],
          run: () => void executeCommand("cm_SwitchToNextTab"),
        }),
        ed.addAction({
          id: "mc.previousTab",
          label: i18n.t("commands.cm_SwitchToPreviousTab"),
          keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Tab],
          run: () => void executeCommand("cm_SwitchToPreviousTab"),
        }),
      );
      if (inActiveGroup(tabId)) ed.focus();
    });
    return () => {
      live = false;
      if (ed) saveViewState(tabId, ed.saveViewState());
      actions.forEach((a) => a.dispose());
      ed?.dispose(); // il modello no: sta in `useEditors`
      instance.current = null;
    };
  }, [tabId, loaded, readOnly]);

  useEffect(() => {
    if (active) instance.current?.focus();
  }, [active]);

  // Con il focus sul body (per esempio dopo un clic sulla barra delle schede) Monaco non riceve il tasto e il comando di
  // tastiera di Ctrl+S (`fileList`) non gira su una scheda di file: lo gestisce questa scheda, se è quella attiva.
  // Col focus altrove (Monaco, una finestra di dialogo, un campo di testo) non si tocca nulla.
  useEffect(() => {
    if (!active) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || document.activeElement !== document.body || chordFromEvent(e) !== "Ctrl+S") return;
      e.preventDefault();
      void useEditors.getState().save(tabId);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, tabId]);

  return (
    <div className="mc-editor" role="region" aria-label={name}>
      {doc && (
        <div className="mc-editor-bar">
          <span className="mc-editor-name" title={path}>{name}</span>
          <span className="mc-editor-meta" title={t("viewer.encodingLabel")}>{doc.encoding}{doc.bom ? " BOM" : ""}</span>
        </div>
      )}
      {readOnly && <div className="mc-editor-note" role="status">{t("editor.readOnly")}</div>}
      {error ? (
        <div className="mc-editor-note" role="alert">
          {error.code === "TOO_LARGE" ? t("editor.tooLarge", { size: formatSize(EDITOR_MAX_BYTES, i18n.language) }) : t(`errors.${error.code}`)}
          {error.path && <span>{error.path}</span>}
        </div>
      ) : (
        <div ref={host} className="mc-editor-host" />
      )}
      {notice && (
        <div className="mc-notice" role="alert">
          {t(`errors.${notice.code}`)}
          {notice.path && <span>{notice.path}</span>}
        </div>
      )}
    </div>
  );
}
