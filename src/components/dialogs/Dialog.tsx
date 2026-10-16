import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useJobs } from "../../state/jobs";
import { useUi } from "../../state/ui";
import { ConflictDialog } from "./ConflictDialog";
import { CopyMoveDialog } from "./CopyMoveDialog";
import { DeleteConfirm } from "./DeleteConfirm";
import { ErrorDialog } from "./ErrorDialog";
import { MaskDialog } from "./MaskDialog";
import { MkdirDialog } from "./MkdirDialog";
import "./dialogs.css";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])';

interface DialogProps {
  title: string;
  /** Esc. */
  onCancel: () => void;
  /** Invio fuori dai pulsanti (un pulsante col focus si attiva da solo). */
  onConfirm?: () => void;
  children: ReactNode;
  actions: ReactNode;
}

/**
 * Finestra modale: prende il focus all'apertura (l'elemento con `data-autofocus`, altrimenti il
 * primo), lo tiene dentro con Tab e lo restituisce alla chiusura. I tasti non escono dalla finestra.
 */
export function Dialog({ title, onCancel, onConfirm, children, actions }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const el = ref.current!;
    const first = el.querySelector<HTMLElement>("[data-autofocus]") ?? el.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? el).focus();
    if (first instanceof HTMLInputElement && first.type === "text") first.select();
    return () => {
      if (before?.isConnected) before.focus();
    };
  }, []);

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
      const items = [...ref.current!.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (!items.length) return;
      const i = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : (i + 1) % items.length;
      e.preventDefault();
      items[next].focus();
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
    ? t("ops.oneItem", { name: paths[0].slice(paths[0].lastIndexOf("\\") + 1) })
    : t("ops.items", { count: paths.length });
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
  }
  if (!prompt) return null;
  return prompt.type === "conflict"
    ? <ConflictDialog key={prompt.promptId} prompt={prompt} />
    : <ErrorDialog key={prompt.promptId} prompt={prompt} />;
}
