import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";

/**
 * Viewing an open channel marks its channel-level posts read (same passive
 * NIP-RS path the standard channel screen uses). topLevelOnly keeps thread
 * replies out of the marker — thread unread clears through what is actually
 * seen: the inline first reply (transcript) and the side chat (panel).
 */
export function useDevReadMarking(
  activeChannel: { id: string; isMember: boolean } | null,
  roots: readonly { created_at: number }[],
): void {
  const { markChannelRead } = useAppShell();
  const latestRootAt =
    roots.length > 0 ? roots[roots.length - 1].created_at : null;
  const activeChannelId = activeChannel?.isMember ? activeChannel.id : null;
  React.useEffect(() => {
    if (!activeChannelId) return;
    markChannelRead(
      activeChannelId,
      latestRootAt === null
        ? null
        : new Date(latestRootAt * 1_000).toISOString(),
      { topLevelOnly: true },
    );
  }, [activeChannelId, latestRootAt, markChannelRead]);
}
