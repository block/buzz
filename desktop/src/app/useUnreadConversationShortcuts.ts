import * as React from "react";

import {
  findUnreadNeighbor,
  readDisplayedConversationOrder,
  type UnreadStepDirection,
} from "@/features/sidebar/lib/unreadConversationOrder";
import { hasActiveEscapeSurface } from "@/shared/hooks/escapeSurfaces";
import { isMacPlatform } from "@/shared/lib/platform";

import {
  requestJumpToUnread,
  requestOpenInboxThreads,
} from "@/app/unreadNavigationEvents";

type KeyChord = Pick<
  KeyboardEvent,
  "altKey" | "code" | "ctrlKey" | "metaKey" | "shiftKey"
>;

/**
 * Previous/next unread conversation. Alt+Shift+Arrows on macOS,
 * Ctrl+Alt+Shift+Arrows on Windows/Linux — plain Alt+Arrows stay reserved for
 * ordinary channel navigation, so the extra Shift (plus Ctrl off-mac, where
 * Alt+Shift+Arrow carries no native text semantics worth preserving) keeps the
 * two namespaces apart.
 */
export function matchUnreadConversationChord(
  event: KeyChord,
  isMac: boolean,
): UnreadStepDirection | null {
  if (event.code !== "ArrowUp" && event.code !== "ArrowDown") return null;
  if (!event.altKey || !event.shiftKey || event.metaKey) return null;
  if (isMac ? event.ctrlKey : !event.ctrlKey) return null;
  return event.code === "ArrowDown" ? "next" : "previous";
}

/**
 * Platform primary modifier plus Shift on the given code. Shift is required:
 * plain Mod+J must keep toggling the terminal.
 */
export function matchModShiftChord(
  event: KeyChord,
  code: string,
  isMac: boolean,
): boolean {
  if (event.code !== code || !event.shiftKey || event.altKey) return false;
  return isMac
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
}

export function useUnreadConversationShortcuts({
  disabled,
  selectedChannelId,
  selectedView,
  unreadChannelIds,
  threadPreviewChannelIds,
  mutedChannelIds,
  onNavigateChannel,
  onNavigateHome,
}: {
  disabled: boolean;
  selectedChannelId: string | null;
  selectedView: string;
  unreadChannelIds: ReadonlySet<string>;
  threadPreviewChannelIds: ReadonlySet<string>;
  mutedChannelIds: ReadonlySet<string>;
  onNavigateChannel: (channelId: string) => void;
  onNavigateHome: () => Promise<unknown>;
}) {
  const stateRef = React.useRef({
    selectedChannelId,
    selectedView,
    unreadChannelIds,
    threadPreviewChannelIds,
    mutedChannelIds,
    onNavigateChannel,
    onNavigateHome,
  });
  stateRef.current = {
    selectedChannelId,
    selectedView,
    unreadChannelIds,
    threadPreviewChannelIds,
    mutedChannelIds,
    onNavigateChannel,
    onNavigateHome,
  };

  React.useEffect(() => {
    if (disabled) return;

    // Capture phase so navigation wins even when a composer or control has
    // focus and stops keydown propagation. Modal dialogs, closable overlay
    // surfaces, and IME composition yield instead of navigating.
    function handleKeyDown(event: KeyboardEvent) {
      const isMac = isMacPlatform();
      const direction = matchUnreadConversationChord(event, isMac);
      const wantsThreads = matchModShiftChord(event, "KeyT", isMac);
      const wantsJump = matchModShiftChord(event, "KeyJ", isMac);
      if (!direction && !wantsThreads && !wantsJump) return;
      if (event.repeat || event.defaultPrevented || event.isComposing) return;
      if (hasActiveEscapeSurface()) return;
      if (document.querySelector('[role="dialog"], [role="alertdialog"]')) {
        return;
      }

      const state = stateRef.current;
      if (direction) {
        const unread = new Set<string>([
          ...state.unreadChannelIds,
          ...state.threadPreviewChannelIds,
        ]);
        const target = findUnreadNeighbor(
          readDisplayedConversationOrder(),
          unread,
          state.mutedChannelIds,
          state.selectedChannelId,
          direction,
        );
        if (!target) return;
        event.preventDefault();
        state.onNavigateChannel(target);
        return;
      }

      if (wantsThreads) {
        event.preventDefault();
        if (state.selectedView === "home") {
          requestOpenInboxThreads();
        } else {
          // HomeView mounts after the navigation commits; the request latches
          // until it consumes the pending open.
          void Promise.resolve()
            .then(() => state.onNavigateHome())
            .then(() => requestOpenInboxThreads());
        }
        return;
      }

      if (
        wantsJump &&
        state.selectedView === "channel" &&
        state.selectedChannelId
      ) {
        event.preventDefault();
        requestJumpToUnread();
      }
    }

    window.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => {
      window.removeEventListener("keydown", handleKeyDown, { capture: true });
    };
  }, [disabled]);
}
