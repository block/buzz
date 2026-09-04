import type * as React from "react";

import { MESSAGE_MARKDOWN_CLASS } from "@/shared/ui/mentionChip";

import {
  BUZZ_COPY_ATTRIBUTE,
  CHANNEL_LABEL_ATTRIBUTE,
  matchChipTextToLabel,
  MENTION_KIND_ATTRIBUTE,
  MENTION_LABEL_ATTRIBUTE,
  MENTION_PUBKEY_ATTRIBUTE,
} from "./mentionClipboard";

/**
 * Selection copy out of the rendered timeline.
 *
 * A rendered mention chip drops the `@` from its DOM text, so a plain browser
 * copy yields dead text ("John Smith") that no composer can re-light. We build
 * both clipboard flavors ourselves from a clone of the selection: sigils
 * restored, identity attributes intact.
 */

/**
 * Off-screen host for the clone. It must actually be rendered — `innerText`
 * falls back to `textContent` (no block newlines) on an unrendered element,
 * which would flatten a multi-message selection onto one line.
 */
const CLONE_HOST_STYLE =
  "position:fixed;top:0;left:-10000px;width:48rem;opacity:0;pointer-events:none;";

/**
 * Restore the sigil each chip strips for display.
 *
 * A chip whose cloned text no longer carries its full label was only partially
 * selected. Prefixing a sigil there would invent a mention the user didn't
 * copy, so the fragment degrades to plain text and its identity attributes are
 * dropped — paste must not register a name from a partial label.
 *
 * The classification is `matchChipTextToLabel`, the same predicate the paste
 * side applies, rather than an equality test against the chip's rendered text:
 * a chip that renders anything but its bare label — an ellipsized long label
 * today — would otherwise read as a fragment, and a selection whose only chip
 * did that would lose its identity and fall back to the browser's dead-text
 * default with nothing to signal it.
 */
function restoreChipSigils(root: HTMLElement): boolean {
  let restored = false;

  for (const element of root.querySelectorAll<HTMLElement>("[data-mention]")) {
    const label = element.getAttribute(MENTION_LABEL_ATTRIBUTE);
    if (
      label &&
      matchChipTextToLabel(element.textContent ?? "", label, "@") !== "fragment"
    ) {
      element.textContent = `@${label}`;
      restored = true;
      continue;
    }
    element.removeAttribute("data-mention");
    element.removeAttribute(MENTION_PUBKEY_ATTRIBUTE);
    element.removeAttribute(MENTION_KIND_ATTRIBUTE);
    element.removeAttribute(MENTION_LABEL_ATTRIBUTE);
  }

  for (const element of root.querySelectorAll<HTMLElement>(
    "[data-channel-link]",
  )) {
    const label = element.getAttribute(CHANNEL_LABEL_ATTRIBUTE);
    if (
      label &&
      matchChipTextToLabel(element.textContent ?? "", label, "#") !== "fragment"
    ) {
      element.textContent = `#${label}`;
      restored = true;
      continue;
    }
    element.removeAttribute("data-channel-link");
    element.removeAttribute(CHANNEL_LABEL_ATTRIBUTE);
  }

  return restored;
}

/** Chip selector matching the inline-flex rules in `globals/markdown.css`. */
const INLINE_CHIP_SELECTOR =
  ".mention-chip, .inline-code-chip, :not(pre) > code";

/**
 * Collapse chip boxes back to plain inline boxes.
 *
 * A chip is a flex container, and a chip inside a profile-popover trigger is
 * also a flex *item*, so it lays out as a block-level box. `innerText` breaks a
 * line around every one of those — "@John Smith\n fixed the bug". Walking up
 * from each chip to its block ancestor and forcing `display: inline` restores
 * the sentence. This runs on the detached clone, so nothing on screen moves.
 */
function inlineChipBoxes(root: HTMLElement): void {
  for (const chip of root.querySelectorAll<HTMLElement>(INLINE_CHIP_SELECTOR)) {
    for (
      let node: HTMLElement | null = chip;
      node && node !== root;
      node = node.parentElement
    ) {
      // A wrapping chip can itself be blockified by its flex trigger. It is
      // still inline message content; only a block ancestor ends this walk.
      if (node !== chip && getComputedStyle(node).display === "block") break;
      node.style.display = "inline";
    }
  }
}

/**
 * Build the `text/plain` + `text/html` flavors for a timeline selection.
 *
 * Returns `null` when the selection carries no chip — an ordinary text copy
 * stays on the browser's own path, which serializes it better than we can.
 */
export function buildTimelineClipboardFlavors(
  selection: Selection | null,
): { text: string; html: string } | null {
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) {
    return null;
  }

  const clone = document.createElement("div");
  // Message styling is scoped to the message-markdown wrapper, so the clone
  // has to carry it for the cloned nodes to lay out the way they do on screen.
  clone.className = MESSAGE_MARKDOWN_CLASS;
  clone.setAttribute("aria-hidden", "true");
  clone.setAttribute("style", CLONE_HOST_STYLE);
  for (let index = 0; index < selection.rangeCount; index += 1) {
    clone.append(selection.getRangeAt(index).cloneContents());
  }

  if (!restoreChipSigils(clone)) return null;

  const html = `<span ${BUZZ_COPY_ATTRIBUTE}="rich">${clone.innerHTML}</span>`;

  // `innerText` is layout-aware, so the plain flavor keeps the block structure
  // of a multi-message selection — with the sigils back and the chips inlined.
  document.body.append(clone);
  let text = "";
  try {
    inlineChipBoxes(clone);
    text = clone.innerText;
  } finally {
    clone.remove();
  }

  return { html, text: text || selection.toString() };
}

/**
 * `onCopy` for any surface that renders message Markdown.
 *
 * Left as a no-op (default copy) for selections without chips, and for
 * copies whose clipboard data the browser withheld.
 */
export function handleTimelineMentionCopy(event: React.ClipboardEvent): void {
  if (event.defaultPrevented) return;
  // React's types claim clipboardData is always present, but the runtime can
  // withhold it — guard before preventDefault(), or the default copy is
  // suppressed and setData throws, leaving the clipboard empty.
  const clipboardData: DataTransfer | null = event.clipboardData;
  if (!clipboardData) return;
  const flavors = buildTimelineClipboardFlavors(window.getSelection());
  if (!flavors) return;
  event.preventDefault();
  clipboardData.setData("text/plain", flavors.text);
  clipboardData.setData("text/html", flavors.html);
}
