import type * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { getThreadReference } from "@/features/messages/lib/threading";
import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";
import { copyTextToClipboard } from "@/shared/lib/clipboard";

// Row-scoped shortcuts leave text input and nested controls untouched.

/** Marks a focusable message row (`<article data-message-focus="true">`). */
export const FOCUSED_MESSAGE_ROW_SELECTOR = '[data-message-focus="true"]';

/** Scopes ArrowUp/Down movement to one conversation surface (`main`/`thread`). */
export const FOCUSED_MESSAGE_LIST_ATTRIBUTE = "data-focused-message-list";

const THREAD_PANEL_TESTID = "message-thread-panel";
const CHANNEL_DROP_ZONE_TESTID = "channel-drop-zone";
const COMPOSER_INPUT_TESTID = "message-input";

/** Copying a message link is offered from the hover action bar, the More menu,
 *  and the `L` keyboard shortcut; all paths share this exact link-building +
 *  toast behavior. */
export function copyMessageLink(channelId: string, message: TimelineMessage) {
  const { rootId } = getThreadReference(message.tags ?? []);
  const link = buildMessageLink({
    channelId,
    messageId: message.id,
    threadRootId: rootId,
  });
  copyTextToClipboard(link, "Link copied to clipboard");
}

/** Gate shared by every copy-link surface: pending sends have no delivered
 *  event to link to, huddle system rows aren't linkable, and callers without
 *  a channelId (e.g. inbox preview rows) can't build the link. */
export function canCopyMessageLink(
  message: TimelineMessage,
  channelId: string | null | undefined,
): channelId is string {
  return (
    !message.pending &&
    message.kind !== KIND_HUDDLE_STARTED &&
    Boolean(channelId)
  );
}

export type FocusedMessageAction =
  | "focus-prev"
  | "focus-next"
  | "reply"
  | "go-back"
  | "to-composer"
  | "react"
  | "edit"
  | "mark-unread"
  | "copy-link";

export type FocusedMessageActionAvailability = {
  canReply: boolean;
  canGoBack: boolean;
  canReact: boolean;
  canEdit: boolean;
  canMarkUnread: boolean;
  canCopyLink: boolean;
};

export type FocusedMessageKeyState = {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
  /** True only when the row itself owns focus (not a nested link/control). */
  rowOwnsFocus: boolean;
  /** True while a dialog, menu, or listbox (e.g. the reaction picker) is open. */
  overlayOpen: boolean;
};

/**
 * Pure key→action decision for a focused message row. Kept side-effect free
 * so the guard matrix (modifiers, IME, focus ownership, overlays,
 * per-action availability) is covered by unit tests; the DOM effects live in
 * `handleFocusedMessageKeyDown` below.
 */
export function resolveFocusedMessageAction(
  event: FocusedMessageKeyState,
  availability: FocusedMessageActionAvailability,
): FocusedMessageAction | null {
  if (event.isComposing || !event.rowOwnsFocus || event.overlayOpen) {
    return null;
  }
  const hasModifier = event.ctrlKey || event.metaKey || event.altKey;
  switch (event.key) {
    case "ArrowUp":
      return event.ctrlKey || event.metaKey || event.altKey || event.shiftKey
        ? null
        : "focus-prev";
    case "ArrowDown":
      return event.ctrlKey || event.metaKey || event.altKey || event.shiftKey
        ? null
        : "focus-next";
    // ArrowLeft/Right with modifiers are left to the browser / app bindings
    // (Alt+arrows are reserved for channel navigation).
    case "ArrowRight":
      return !hasModifier && !event.shiftKey && availability.canReply
        ? "reply"
        : null;
    case "ArrowLeft":
      return !hasModifier && !event.shiftKey && availability.canGoBack
        ? "go-back"
        : null;
    case "Escape":
      return hasModifier || event.shiftKey ? null : "to-composer";
    default:
      break;
  }
  // Single letters fire without Ctrl/Meta/Alt (Shift is folded by
  // lower-casing, so `T` and `t` both reply). Anything else — Enter, Space,
  // Tab, multi-char keys — is left alone.
  if (hasModifier || event.key.length !== 1) {
    return null;
  }
  switch (event.key.toLowerCase()) {
    case "t":
      return availability.canReply ? "reply" : null;
    case "r":
      return availability.canReact ? "react" : null;
    case "e":
      return availability.canEdit ? "edit" : null;
    case "u":
      return availability.canMarkUnread ? "mark-unread" : null;
    case "l":
      return availability.canCopyLink ? "copy-link" : null;
    default:
      return null;
  }
}

/** F6 from the composer claims timeline focus (even with a non-empty draft,
 *  which is left untouched). Alt/Ctrl/Meta, IME, autocomplete, and open
 *  overlays all decline so app and channel-navigation bindings keep the key. */
export function shouldClaimComposerTimelineFocus(event: {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  repeat: boolean;
  isComposing: boolean;
  autocompleteOpen: boolean;
  overlayOpen: boolean;
}): boolean {
  return (
    event.key === "F6" &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.altKey &&
    !event.shiftKey &&
    !event.repeat &&
    !event.isComposing &&
    !event.autocompleteOpen &&
    !event.overlayOpen
  );
}

function queryScopeRows(scope: ParentNode): HTMLElement[] {
  return [...scope.querySelectorAll<HTMLElement>(FOCUSED_MESSAGE_ROW_SELECTOR)];
}

function focusRow(row: HTMLElement) {
  row.focus({ preventScroll: true });
  // `nearest` keeps the focused row visible without yanking the reader's
  // scroll position; in the virtualized timeline this stays inside mounted
  // rows because movement is always to an adjacent row.
  row.scrollIntoView({ block: "nearest" });
}

/** True while a dialog, menu, or listbox overlay (reaction picker, dropdown,
 *  confirm dialog, autocomplete) is open somewhere in the document. */
export function isFocusOverlayOpen(): boolean {
  if (typeof document === "undefined") return false;
  return (
    document.querySelector(
      '[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]',
    ) !== null
  );
}

/** Focus the adjacent focusable row within the same conversation surface. */
export function focusNeighborRow(
  current: HTMLElement,
  direction: 1 | -1,
): boolean {
  const scope =
    current.closest(`[${FOCUSED_MESSAGE_LIST_ATTRIBUTE}]`) ?? document;
  const rows = queryScopeRows(scope);
  const next = rows[rows.indexOf(current) + direction];
  if (!next) return false;
  focusRow(next);
  return true;
}

/**
 * Explicit keyboard entry from the composer: focus the newest focusable row
 * in the composer's own surface (thread panel rows for the thread composer,
 * main timeline rows otherwise). Never modifies the draft.
 */
export function focusLastRowForElement(anchor: Element): boolean {
  const scope =
    anchor.closest(`[data-testid="${THREAD_PANEL_TESTID}"]`) ??
    anchor.closest(`[data-testid="${CHANNEL_DROP_ZONE_TESTID}"]`) ??
    document;
  const rows = queryScopeRows(scope);
  const last = rows[rows.length - 1];
  if (!last) return false;
  focusRow(last);
  return true;
}

/** Escape from a focused row: return focus to that surface's composer. */
export function focusComposerForRow(row: HTMLElement): boolean {
  const scope =
    row.closest(`[data-testid="${THREAD_PANEL_TESTID}"]`) ??
    row.closest(`[data-testid="${CHANNEL_DROP_ZONE_TESTID}"]`) ??
    document;
  const input = scope.querySelector<HTMLElement>(
    `[data-testid="${COMPOSER_INPUT_TESTID}"]`,
  );
  if (!input) return false;
  input.focus();
  return true;
}

/** `R` opens the row's existing reaction picker via its own trigger, reusing
 *  the action bar's picker, toggle, and error handling unchanged. */
export function openReactionPickerForRow(
  row: HTMLElement,
  messageId: string,
): boolean {
  const escapeId =
    typeof CSS !== "undefined" && typeof CSS.escape === "function"
      ? CSS.escape
      : (value: string) => value;
  const trigger = row.querySelector<HTMLElement>(
    `[data-testid="react-message-${escapeId(messageId)}"]`,
  );
  if (!trigger) return false;
  trigger.click();
  return true;
}

export type FocusedMessageKeyHandlers = {
  channelId?: string | null;
  message: TimelineMessage;
  onReply?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onNavigateBack?: () => void;
  canOpenReactions: boolean;
};

/**
 * Row-level `onKeyDown` for a focused message. Returns true when the key was
 * claimed (the event is already `preventDefault`ed). All action callbacks are
 * the row's existing ones, including edit authorization (call sites only pass
 * `onEdit` for manageable messages) and reaction availability.
 */
export function handleFocusedMessageKeyDown(
  event: React.KeyboardEvent<HTMLElement>,
  handlers: FocusedMessageKeyHandlers,
): boolean {
  if (event.defaultPrevented) return false;
  const { message } = handlers;
  const action = resolveFocusedMessageAction(
    {
      key: event.key,
      ctrlKey: event.ctrlKey,
      metaKey: event.metaKey,
      altKey: event.altKey,
      shiftKey: event.shiftKey,
      isComposing: event.nativeEvent?.isComposing ?? false,
      rowOwnsFocus: event.target === event.currentTarget,
      overlayOpen: isFocusOverlayOpen(),
    },
    {
      canReply: handlers.onReply !== undefined,
      canGoBack: handlers.onNavigateBack !== undefined,
      canReact: handlers.canOpenReactions,
      canEdit: handlers.onEdit !== undefined,
      canMarkUnread: handlers.onMarkUnread !== undefined,
      canCopyLink: canCopyMessageLink(message, handlers.channelId),
    },
  );
  if (action === null) return false;
  const row = event.currentTarget;
  switch (action) {
    case "focus-prev":
      if (!focusNeighborRow(row, -1)) return false;
      break;
    case "focus-next":
      if (!focusNeighborRow(row, 1)) return false;
      break;
    case "reply":
      handlers.onReply?.(message);
      break;
    case "go-back":
      handlers.onNavigateBack?.();
      break;
    case "to-composer":
      if (!focusComposerForRow(row)) return false;
      break;
    case "react":
      if (!openReactionPickerForRow(row, message.id)) return false;
      break;
    case "edit":
      handlers.onEdit?.(message);
      break;
    case "mark-unread":
      // Marking unread can remount timeline rows; keep focus in the composer.
      focusComposerForRow(row);
      handlers.onMarkUnread?.(message);
      break;
    case "copy-link":
      if (handlers.channelId === undefined || handlers.channelId === null) {
        return false;
      }
      copyMessageLink(handlers.channelId, message);
      break;
  }
  event.preventDefault();
  return true;
}
