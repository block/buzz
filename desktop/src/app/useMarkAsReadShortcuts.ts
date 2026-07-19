import * as React from "react";

import { hasActiveEscapeSurface } from "@/shared/hooks/escapeSurfaces";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

export function useMarkAsReadShortcuts({
  activeChannelId,
  activeChannelLastMessageAt,
  markAllChannelsRead,
  markChannelRead,
  selectedView,
}: {
  activeChannelId: string | null;
  activeChannelLastMessageAt: string | null | undefined;
  markAllChannelsRead: () => void;
  markChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  selectedView: string;
}) {
  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (event.defaultPrevented) return;
      if (hasPrimaryShortcutModifier(event) || event.altKey) return;
      // A closable foreground surface (focus drawer, overlay panel,
      // single-panel thread — anything holding `useEscapeKey`) owns Escape
      // while open: Escape means "close that", never "mark the channel I
      // can't currently see as read". Window listeners fire in registration
      // order, so without this yield the app-mount listener would win the
      // key over the surface that opened later.
      if (hasActiveEscapeSurface()) return;

      if (event.shiftKey) {
        event.preventDefault();
        markAllChannelsRead();
        return;
      }

      if (selectedView === "channel" && activeChannelId) {
        event.preventDefault();
        markChannelRead(activeChannelId, activeChannelLastMessageAt ?? null);
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    activeChannelId,
    activeChannelLastMessageAt,
    markAllChannelsRead,
    markChannelRead,
    selectedView,
  ]);
}
