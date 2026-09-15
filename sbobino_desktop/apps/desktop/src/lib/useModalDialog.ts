import { useLayoutEffect, useRef } from "react";

export function useModalDialog(open: boolean) {
  const ref = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!open || !dialog) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const focusableSelector =
      "button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])";
    const focusFirst = () =>
      dialog.querySelector<HTMLElement>(focusableSelector)?.focus();
    const keepFocusInside = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const focusable = [...dialog.querySelectorAll<HTMLElement>(focusableSelector)];
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (!dialog.contains(document.activeElement) || (event.shiftKey && document.activeElement === first)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    dialog.showModal?.();
    focusFirst();
    const focusTimer = window.setTimeout(focusFirst);
    document.addEventListener("keydown", keepFocusInside, true);
    return () => {
      window.clearTimeout(focusTimer);
      document.removeEventListener("keydown", keepFocusInside, true);
      dialog.close?.();
      previouslyFocused?.focus();
    };
  }, [open]);
  return ref;
}
