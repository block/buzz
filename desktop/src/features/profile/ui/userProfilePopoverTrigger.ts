// The popover trigger wraps its children. When the profile panel can open, the
// wrapper takes role="button" + tabIndex so it is clickable and focusable. Most
// call sites pass a real <button> child, which is already focusable, so the
// wrapper would add a second Tab stop for one control (block/buzz#2394). These
// helpers decide whether the wrapper should own focus, which it should only when
// the child is not itself a focusable element.

type TriggerChild =
  | { type: unknown; props: Record<string, unknown> }
  | null
  | undefined;

const INTERACTIVE_ELEMENTS = new Set(["button", "select", "textarea"]);

// True when the resolved single child is itself in the tab order, so the wrapper
// should defer to it. Only intrinsic elements are inspectable; a component child
// is treated as non-focusable so the wrapper keeps its own focus behavior.
export function isInteractiveTriggerChild(child: TriggerChild): boolean {
  if (!child || typeof child.type !== "string") {
    return false;
  }
  // An explicit tabIndex decides focusability for any element: >= 0 is a Tab
  // stop, < 0 is removed from the tab order.
  if (typeof child.props.tabIndex === "number") {
    return child.props.tabIndex >= 0;
  }
  if (child.type === "a") {
    return child.props.href != null;
  }
  if (child.type === "input") {
    return child.props.type !== "hidden" && child.props.disabled !== true;
  }
  if (INTERACTIVE_ELEMENTS.has(child.type)) {
    // Form controls drop out of the tab order when disabled.
    return child.props.disabled !== true;
  }
  return false;
}

// Whether the trigger wrapper should carry its own role/tabIndex/onKeyDown: only
// when the panel can open and the child does not already provide a focusable,
// keyboard-activatable element.
export function triggerShouldOwnFocus(
  child: TriggerChild,
  canOpenProfilePanel: boolean,
): boolean {
  return canOpenProfilePanel && !isInteractiveTriggerChild(child);
}
