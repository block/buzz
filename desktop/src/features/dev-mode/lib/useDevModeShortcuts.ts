import * as React from "react";

import type { Channel } from "@/shared/api/types";

/**
 * Window-level developer mode shortcuts:
 *
 * - ⌃O toggles the command palette
 * - ⌘N jumps to the fresh composer (new channel)
 * - ⌘T drafts a side chat in the open channel
 * - ⌘⇧T drafts a new tab (sub-channel) of the open main
 * - ⌘[/⌘] cycle through the open channel's tabs, wrapping at the ends
 */
export function useDevModeShortcuts({
  view,
  activeChannel,
  activeMainChannel,
  activeSubChannels,
  onTogglePalette,
  onNewSession,
  onDraftSideChat,
  onDraftTab,
  onOpenChannel,
}: {
  view: "fresh" | "navigator" | "channel";
  activeChannel: Channel | null;
  activeMainChannel: Channel | null;
  activeSubChannels: Channel[];
  onTogglePalette: () => void;
  onNewSession: () => void;
  /** Null when the current view has no open channel to side-chat in. */
  onDraftSideChat: (() => void) | null;
  /** Null when the current view has no main channel to spawn a tab of. */
  onDraftTab: (() => void) | null;
  onOpenChannel: (channelId: string) => void;
}) {
  React.useEffect(() => {
    const handleWindowKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && !event.metaKey && event.key.toLowerCase() === "o") {
        event.preventDefault();
        onTogglePalette();
        return;
      }
      if (!event.metaKey || event.ctrlKey || event.altKey) return;
      const key = event.key.toLowerCase();
      if (event.shiftKey) {
        if (key === "t" && onDraftTab) {
          event.preventDefault();
          onDraftTab();
        }
        return;
      }
      if (key === "n") {
        event.preventDefault();
        onNewSession();
      } else if (key === "t" && onDraftSideChat) {
        event.preventDefault();
        onDraftSideChat();
      } else if (
        (event.key === "[" || event.key === "]") &&
        view === "channel" &&
        activeChannel &&
        activeMainChannel
      ) {
        event.preventDefault();
        const tabs = [activeMainChannel, ...activeSubChannels];
        if (tabs.length < 2) return;
        const index = tabs.findIndex((tab) => tab.id === activeChannel.id);
        const direction = event.key === "]" ? 1 : -1;
        onOpenChannel(tabs[(index + direction + tabs.length) % tabs.length].id);
      }
    };
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => window.removeEventListener("keydown", handleWindowKeyDown);
  }, [
    activeChannel,
    activeMainChannel,
    activeSubChannels,
    onDraftSideChat,
    onDraftTab,
    onNewSession,
    onOpenChannel,
    onTogglePalette,
    view,
  ]);
}
