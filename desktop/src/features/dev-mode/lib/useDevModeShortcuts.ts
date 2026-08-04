import * as React from "react";

import type { Channel } from "@/shared/api/types";

/**
 * Window-level developer mode shortcuts:
 *
 * - ⌘K toggles the command palette
 * - ⌘N jumps to the fresh composer (new channel)
 * - ⌘T drafts a side chat in the open channel
 * - ⌘⇧T drafts a new tab (sub-channel) of the open main
 * - ⇧⌘[/⇧⌘] cycle through the open channel's tabs, wrapping at the ends
 * - ⌘1–⌘8 jump to that tab (main is 1); ⌘9 jumps to the last tab
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
    // ⌘K must beat the standard UI's global ⌘K search binding (a window
    // bubble listener that yields when `event.defaultPrevented`), so the
    // palette toggle listens in the capture phase.
    const handlePaletteKeyDown = (event: KeyboardEvent) => {
      if (
        event.metaKey &&
        !event.ctrlKey &&
        !event.altKey &&
        !event.shiftKey &&
        event.key.toLowerCase() === "k"
      ) {
        event.preventDefault();
        onTogglePalette();
      }
    };
    const handleWindowKeyDown = (event: KeyboardEvent) => {
      if (!event.metaKey || event.ctrlKey || event.altKey) return;
      const key = event.key.toLowerCase();
      if (event.shiftKey) {
        if (key === "t" && onDraftTab) {
          event.preventDefault();
          onDraftTab();
        } else if (
          (event.code === "BracketLeft" || event.code === "BracketRight") &&
          view === "channel" &&
          activeChannel &&
          activeMainChannel
        ) {
          event.preventDefault();
          const tabs = [activeMainChannel, ...activeSubChannels];
          if (tabs.length < 2) return;
          const index = tabs.findIndex((tab) => tab.id === activeChannel.id);
          const direction = event.code === "BracketRight" ? 1 : -1;
          onOpenChannel(
            tabs[(index + direction + tabs.length) % tabs.length].id,
          );
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
        /^Digit[1-9]$/.test(event.code) &&
        view === "channel" &&
        activeMainChannel
      ) {
        // Browser-style: ⌘1–⌘8 pick that tab (main is 1), ⌘9 the last.
        event.preventDefault();
        const tabs = [activeMainChannel, ...activeSubChannels];
        const digit = Number(event.code.slice(-1));
        const tab = digit === 9 ? tabs[tabs.length - 1] : tabs[digit - 1];
        if (tab) onOpenChannel(tab.id);
      }
    };
    window.addEventListener("keydown", handlePaletteKeyDown, true);
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => {
      window.removeEventListener("keydown", handlePaletteKeyDown, true);
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
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
