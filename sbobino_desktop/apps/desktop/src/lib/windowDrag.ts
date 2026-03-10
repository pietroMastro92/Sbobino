const WINDOW_DRAG_IGNORE_PARTS = [
  "button",
  "input",
  "select",
  "textarea",
  "a",
  "label",
  "audio",
  "canvas",
  "summary",
  "video",
  "[role=\"button\"]",
  "[role=\"menuitem\"]",
  "[role=\"switch\"]",
  "[tabindex]",
  "[contenteditable=\"true\"]",
  "[data-window-drag-ignore]",
  "[data-tauri-drag-region=\"false\"]",
];

export const WINDOW_DRAG_IGNORE_SELECTOR = WINDOW_DRAG_IGNORE_PARTS.join(", ");
export const WINDOW_DRAG_AREA_SELECTOR = [
  "[data-window-drag-area]",
  "[data-tauri-drag-region]:not([data-tauri-drag-region=\"false\"])",
].join(", ");

type WindowDragOptions = {
  requireExplicitArea?: boolean;
};

export function shouldStartWindowDrag(target: EventTarget | null, options?: WindowDragOptions): boolean {
  if (!(target instanceof Element) || target.closest(WINDOW_DRAG_IGNORE_SELECTOR)) {
    return false;
  }

  if (options?.requireExplicitArea && !target.closest(WINDOW_DRAG_AREA_SELECTOR)) {
    return false;
  }

  return true;
}
