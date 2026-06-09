import * as React from "react";

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
