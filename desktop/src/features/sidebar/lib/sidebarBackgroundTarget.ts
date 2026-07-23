const INTERACTIVE_SIDEBAR_TARGET_SELECTOR = [
  "a",
  "button",
  "input",
  "select",
  "textarea",
  "[contenteditable='true']",
  "[role='button']",
  "[role='link']",
  "[role='menuitem']",
  "[role='option']",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

/** Whether a sidebar click belongs to a control rather than its background. */
export function isInteractiveSidebarTarget(
  target: EventTarget | null,
): boolean {
  const element =
    typeof Element !== "undefined" && target instanceof Element
      ? target
      : typeof Node !== "undefined" && target instanceof Node
        ? target.parentElement
        : null;
  return Boolean(element?.closest(INTERACTIVE_SIDEBAR_TARGET_SELECTOR) ?? null);
}
