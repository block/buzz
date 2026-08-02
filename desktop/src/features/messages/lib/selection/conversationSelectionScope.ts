/**
 * Pure selection-scoping logic for the conversation panes.
 *
 * Bug (block/buzz#4077): in split view a native drag-selection can escape the
 * conversation pane where it started, pulling in content from a sibling pane
 * (channel and thread at once), the hover quick-reaction bar, inline reaction
 * pills, emoji pickers, popovers, and drop-zone overlays. The reporter's
 * expected semantics:
 *
 *   - Channel selection stays inside the channel.
 *   - Thread selection stays inside the thread.
 *   - Message text remains copyable.
 *   - Hover action bars, quick reactions, reaction pills, emoji pickers,
 *     reaction popovers, and other interactive overlays are excluded.
 *   - Timestamps and ordinary message metadata may remain selectable.
 *
 * The mechanism is a `selectionchange` listener on the shared conversation
 * root that, once a selection is "anchored" to a pane, collapses the focus
 * end back to the pane's boundary whenever the user drags past it. This file
 * holds only the *pure* decisions — which elements are out of scope, which
 * pane an element belongs to, where to clamp — expressed against the real DOM
 * so tests can supply structural doubles without jsdom.
 */

// ---------------------------------------------------------------------------
// Markers
// ---------------------------------------------------------------------------

/** Attribute placed on each conversation pane root (channel, thread, aux). */
export const SELECTION_PANE_ATTR = "data-selection-pane";

/**
 * Attribute placed on interactive chrome that must never participate in a
 * native text selection (hover action bars, quick reactions, reaction pills,
 * emoji pickers, popovers, drop-zone overlays).
 */
export const SELECTION_EXCLUDE_ATTR = "data-selection-exclude";

/** Selector that finds the nearest pane ancestor of an element. */
export const PANE_SELECTOR = `[${SELECTION_PANE_ATTR}]`;

/**
 * Selector matching surfaces that must own their selection — text inputs,
 * textareas, contenteditables. When the selection anchor/focus sits inside
 * one of these we never interfere (composer + IME must be left alone).
 */
export const INTERACTIVE_TEXT_SELECTOR = [
  "input",
  "textarea",
  "[contenteditable]",
  "[contenteditable='true']",
].join(",");

/**
 * Selector for portalled dialog/popover overlays (Radix mounts these to
 * `body`, outside the pane tree). A selection moving into one must not be
 * dragged back into a conversation pane.
 */
export const OVERLAY_SELECTOR = [
  "[data-radix-dialog]",
  "[role='dialog']",
  "[data-radix-popper-content-wrapper]",
].join(",");

/** Selector for chrome excluded from selection (mirrors the marker). */
export const EXCLUDE_SELECTOR = `[${SELECTION_EXCLUDE_ATTR}]`;

// ---------------------------------------------------------------------------
// Pure predicates
// ---------------------------------------------------------------------------

/** Return the nearest ancestor (or self) carrying the pane attribute. */
export function closestPane(element: Element | null | undefined): Element | null {
  if (!element) return null;
  return element.closest(PANE_SELECTOR);
}

/** True when the element sits inside an excluded chrome region. */
export function isExcludedChrome(element: Element | null | undefined): boolean {
  if (!element) return false;
  return element.closest(EXCLUDE_SELECTOR) !== null;
}

/** True when the element sits inside an interactive text/editing surface. */
export function isWithinInteractiveText(
  element: Element | null | undefined,
): boolean {
  if (!element) return false;
  return element.closest(INTERACTIVE_TEXT_SELECTOR) !== null;
}

/** True when the element sits inside a portalled dialog/popover overlay. */
export function isWithinOverlay(element: Element | null | undefined): boolean {
  if (!element) return false;
  return element.closest(OVERLAY_SELECTOR) !== null;
}

/** True when `element` is `pane` or a descendant of `pane`. */
export function isElementWithinPane(
  element: Element | null | undefined,
  pane: Element | null | undefined,
): boolean {
  if (!element || !pane) return false;
  if (element === pane) return true;
  return pane.contains(element);
}

// DOM Node constants replicated so tests run without a real Node global.
export const ELEMENT_NODE = 1;
export const TEXT_NODE = 3;
export const DOCUMENT_POSITION_FOLLOWING = 0x04;

/**
 * Decide whether a selection whose focus moved to `focusElement` should be
 * clamped back to `anchorPane`.
 *
 * We clamp only when:
 *   - An anchor pane exists (the drag began inside a conversation pane).
 *   - The focus has genuinely left that pane.
 *   - The focus is NOT inside an interactive text surface (composers own
 *     their own selection — an IME composition must never be disturbed).
 *   - The focus is NOT inside a top-level dialog/popover overlay.
 *
 * Returning the anchor pane signals "clamp"; returning `null` signals
 * "leave the selection alone".
 */
export function shouldClampSelectionToPane(
  anchorPane: Element | null | undefined,
  focusElement: Element | null | undefined,
): Element | null {
  if (!anchorPane) return null;
  if (!focusElement) return null;
  if (isElementWithinPane(focusElement, anchorPane)) return null;
  if (isWithinInteractiveText(focusElement)) return null;
  if (isWithinOverlay(focusElement)) return null;
  return anchorPane;
}

// ---------------------------------------------------------------------------
// Clamp target resolution
// ---------------------------------------------------------------------------

/**
 * A directional boundary hit describes whether the user dragged out below the
 * pane (`"after"`) or above it (`"before"`), so the clamp can target the
 * correct edge.
 */
export type PaneBoundary = "before" | "after";

/**
 * Given the anchor pane and the element the focus escaped toward, choose the
 * pane boundary to clamp to. We compare document positions; the fallback is
 * `"after"` (the overwhelmingly common downward drag).
 */
export function resolveClampBoundary(
  anchorPane: Element | null | undefined,
  focusElement: Element | null | undefined,
): PaneBoundary {
  if (!anchorPane || !focusElement) return "after";
  const position = focusElement.compareDocumentPosition(anchorPane);
  // DOCUMENT_POSITION_FOLLOWING means anchorPane follows focusElement,
  // i.e. focusElement is before the pane → user dragged above the pane.
  if ((position & DOCUMENT_POSITION_FOLLOWING) !== 0) return "before";
  return "after";
}
