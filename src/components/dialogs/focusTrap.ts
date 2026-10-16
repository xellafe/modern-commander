import { useEffect, type KeyboardEvent, type RefObject } from "react";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * Focus di una finestra modale: all'apertura va all'elemento con `data-autofocus` (altrimenti al primo
 * focalizzabile) e alla chiusura torna a chi lo aveva.
 */
export function useModalFocus(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const el = ref.current!;
    const first = el.querySelector<HTMLElement>("[data-autofocus]") ?? el.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? el).focus();
    if (first instanceof HTMLInputElement && first.type === "text") first.select();
    return () => {
      if (before?.isConnected) before.focus();
    };
  }, [ref]);
}

/** Tab e Maiusc+Tab restano dentro `root`: dopo l'ultimo elemento si torna al primo. */
export function trapTab(e: KeyboardEvent, root: HTMLElement) {
  const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)];
  if (!items.length) return;
  const i = items.indexOf(document.activeElement as HTMLElement);
  const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : (i + 1) % items.length;
  e.preventDefault();
  items[next].focus();
}
