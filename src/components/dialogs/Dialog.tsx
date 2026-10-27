import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { folderName } from "../../lib/paths";
import { useJobs } from "../../state/jobs";
import { useUi, type DialogRequest } from "../../state/ui";
import { ConflictDialog } from "./ConflictDialog";
import { ConnectionsDialog } from "./ConnectionsDialog";
import { CopyMoveDialog } from "./CopyMoveDialog";
import { DeleteConfirm } from "./DeleteConfirm";
import { ErrorDialog, UnsafeWriteDialog } from "./ErrorDialog";
import { ExitConfirm } from "./ExitConfirm";
import { MaskDialog } from "./MaskDialog";
import { MkdirDialog } from "./MkdirDialog";
import { PasswordDialog } from "./PasswordDialog";
import { SaveErrorDialog } from "./SaveErrorDialog";
import { TrustDialog } from "./TrustDialog";
import { UnsavedDialog } from "./UnsavedDialog";
import { trapTab, useModalFocus } from "./focusTrap";
import "./dialogs.css";

interface DialogProps {
  title: string;
  /** Esc. */
  onCancel: () => void;
  /** Invio fuori dai pulsanti (un pulsante col focus si attiva da solo). */
  onConfirm?: () => void;
  children: ReactNode;
  actions: ReactNode;
}

const ARROWS: Record<string, number> = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 };

/**
 * Finestra modale: prende il focus all'apertura (l'elemento con `data-autofocus`, altrimenti il
 * primo), lo tiene dentro con Tab e lo restituisce alla chiusura. I tasti non escono dalla finestra.
 */
export function Dialog({ title, onCancel, onConfirm, children, actions }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  useModalFocus(ref);

  // Si fermano solo i tasti gestiti qui o dai figli (`preventDefault`): gli altri arrivano a useKeyboard,
  // che col focus nella finestra esegue solo i comandi `global` (Alt+F4…).
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.defaultPrevented || ["Escape", "Enter", "Tab"].includes(e.key)) e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    } else if (e.key === "Enter" && onConfirm && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault();
      onConfirm();
    } else if (e.key === "Tab") {
      trapTab(e, ref.current!);
    } else if (!e.defaultPrevented && e.target instanceof HTMLButtonElement && ARROWS[e.key]) {
      // Come nei messaggi di Windows: le frecce passano al pulsante accanto, fra quelli dello stesso gruppo.
      const row = [...e.target.parentElement!.children].filter(
        (b): b is HTMLButtonElement => b instanceof HTMLButtonElement && !b.disabled && b.getAttribute("aria-disabled") !== "true",
      );
      const i = row.indexOf(e.target);
      if (i < 0 || row.length < 2) return;
      e.preventDefault();
      e.stopPropagation();
      row[(i + ARROWS[e.key] + row.length) % row.length].focus();
    }
  };

  return (
    // Lo sfondo blocca il mouse sul resto della finestra e non ruba il focus.
    <div className="mc-dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && e.preventDefault()}>
      <div ref={ref} className="mc-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} onKeyDown={onKeyDown}>
        <div className="mc-dialog-title" id={titleId}>{title}</div>
        <div className="mc-dialog-body">{children}</div>
        <div className="mc-dialog-actions">{actions}</div>
      </div>
    </div>
  );
}

/** «nome» per un solo elemento, altrimenti "N elementi". */
export function useWhat(paths: string[]): string {
  const { t } = useTranslation();
  return paths.length === 1
    ? t("ops.oneItem", { name: folderName(paths[0]) })
    : t("ops.items", { count: paths.length });
}

/** Avviso con il solo OK. */
function MessageDialog({ req }: { req: Extract<DialogRequest, { kind: "message" }> }) {
  const { t } = useTranslation();
  const close = useUi.getState().closeDialog;
  return (
    <Dialog title={t(req.title)} onCancel={close} actions={<button className="primary" onClick={close}>{t("ops.ok")}</button>}>
      <p>{t(req.text, req.values)}</p>
    </Dialog>
  );
}

/** Una finestra alla volta: prima quella aperta da un comando, poi i prompt della coda (il più vecchio). */
export function DialogHost() {
  const dialog = useUi((s) => s.dialog);
  const prompt = useJobs((s) => s.prompts[0]);
  switch (dialog?.kind) {
    case "copyMove": return <CopyMoveDialog req={dialog} />;
    case "mkdir": return <MkdirDialog req={dialog} />;
    case "delete": return <DeleteConfirm req={dialog} />;
    case "mask": return <MaskDialog req={dialog} />;
    case "exit": return <ExitConfirm req={dialog} />;
    case "unsaved": return <UnsavedDialog req={dialog} />;
    case "saveError": return <SaveErrorDialog req={dialog} />;
    case "connections": return <ConnectionsDialog req={dialog} />;
    case "password": return <PasswordDialog req={dialog} />;
    case "trust": return <TrustDialog req={dialog} />;
    case "message": return <MessageDialog req={dialog} />;
  }
  if (!prompt) return null;
  switch (prompt.type) {
    case "conflict": return <ConflictDialog key={prompt.promptId} prompt={prompt} />;
    case "unsafeWrite": return <UnsafeWriteDialog key={prompt.promptId} prompt={prompt} />;
    default: return <ErrorDialog key={prompt.promptId} prompt={prompt} />;
  }
}
