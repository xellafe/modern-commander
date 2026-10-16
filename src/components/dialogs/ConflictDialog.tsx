import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { nameError } from "../../commands/fileOps";
import type { AppError, Meta, Prompt, PromptReply } from "../../ipc/types";
import { formatDate, formatSize, splitName } from "../../lib/format";
import { conflictReply, renamePath, useJobs, type ConflictChoice } from "../../state/jobs";
import { parentPath } from "../../state/tabView";
import { Dialog } from "./Dialog";

type ConflictPrompt = Extract<Prompt, { type: "conflict" }>;

/** Una risposta alla volta; un rifiuto del backend resta visibile nella finestra. */
export function useAnswer(prompt: Prompt) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const send = async (reply: PromptReply) => {
    if (busy) return;
    setBusy(true);
    try {
      await useJobs.getState().answer(prompt, reply);
    } catch (e) {
      setError(t(`errors.${(e as Partial<AppError>)?.code ?? "IO"}`));
      setBusy(false);
    }
  };
  return { busy, error, setError, send };
}

function FileInfo({ label, path, meta }: { label: string; path: string; meta: Meta }) {
  const { t, i18n } = useTranslation();
  return (
    <div className="mc-conflict-file">
      <span className="mc-field-label">{label}</span>
      <span className="mc-path">{path}</span>
      <span className="mc-muted">
        {t("ops.sizeDate", { size: formatSize(meta.size, i18n.language), date: formatDate(meta.modifiedMs, i18n.language) })}
      </span>
    </div>
  );
}

/** Destinazione esistente: sovrascrivi (non sullo stesso file), salta, rinomina, per data/dimensione, annulla. */
export function ConflictDialog({ prompt }: { prompt: ConflictPrompt }) {
  const { t } = useTranslation();
  const { busy, error, setError, send } = useAnswer(prompt);
  const [all, setAll] = useState(false);
  const suggested = prompt.suggestedName.slice(prompt.suggestedName.lastIndexOf("\\") + 1);
  const [name, setName] = useState(suggested);
  const input = useRef<HTMLInputElement>(null);

  // Si seleziona il nome senza estensione, come nella rinomina in place.
  useEffect(() => {
    if (prompt.sameFile) input.current?.setSelectionRange(0, splitName(suggested, false)[0].length);
  }, []);

  const choose = (c: ConflictChoice) => {
    if (c === "rename") {
      const e = nameError(name.trim());
      if (e) return setError(t(e));
    }
    void send(conflictReply(c, all, renamePath(prompt.suggestedName, name.trim())));
  };
  const button = (c: ConflictChoice, key: string, autofocus = false) => (
    <button disabled={busy} onClick={() => choose(c)} data-autofocus={autofocus || undefined}>{t(key)}</button>
  );

  return (
    <Dialog
      title={t("ops.conflictTitle")}
      onCancel={() => choose("cancel")}
      actions={<>
        {!prompt.sameFile && button("overwrite", "ops.overwrite")}
        {button("skip", "ops.skip", !prompt.sameFile)}
        {button("rename", "ops.rename")}
        {button("newer", "ops.overwriteIfNewer")}
        {button("larger", "ops.overwriteIfLarger")}
        {button("cancel", "ops.cancel")}
      </>}
    >
      <p>{t(prompt.sameFile ? "ops.conflictSameFile" : "ops.conflictExists")}</p>
      <FileInfo label={t("ops.source")} path={prompt.src} meta={prompt.srcMeta} />
      <FileInfo label={t("ops.target")} path={prompt.dst} meta={prompt.dstMeta} />
      <label className="mc-field">
        <span>{t("ops.newName")}</span>
        <span className="mc-rename-row">
          <span className="mc-muted mc-path">{parentPath(prompt.suggestedName)}</span>
          <input
            ref={input}
            type="text"
            value={name}
            spellCheck={false}
            data-autofocus={prompt.sameFile || undefined}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter") return;
              e.preventDefault();
              choose("rename");
            }}
          />
        </span>
      </label>
      <label className="mc-check">
        <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} />
        {t("ops.applyToAll")}
      </label>
      {error && <div className="mc-field-error" role="alert">{error}</div>}
    </Dialog>
  );
}
