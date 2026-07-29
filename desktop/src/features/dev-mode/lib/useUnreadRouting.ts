import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import type { SubChannelIndex } from "@/features/dev-mode/lib/subChannels";
import { findUnreadFamilyTarget } from "@/features/dev-mode/lib/unreadThreads";
import type { RelayEvent } from "@/shared/api/types";

/**
 * Opening a channel from its navigator row routes to what needs attention:
 * the family's unread tab, and inside it the newest unread thread's side
 * chat. The target thread's root event may not be loaded when the channel
 * opens, so the jump is deferred until the roots query delivers it; any
 * navigation elsewhere makes the deferred jump stale.
 *
 * Returns the routed open. Explicit destinations (tab clicks, palette,
 * ⌘[/⌘], ⌥↑↓) keep using the plain open and land exactly where asked.
 */
export function useUnreadRouting({
  subIndex,
  unreadChannelIds,
  topLevelUnreadChannelIds,
  activeChannelId,
  roots,
  openChannel,
  openThread,
}: {
  subIndex: SubChannelIndex;
  unreadChannelIds: ReadonlySet<string>;
  topLevelUnreadChannelIds: ReadonlySet<string>;
  /** The channel whose transcript (roots) is currently loaded. */
  activeChannelId: string | null;
  roots: readonly RelayEvent[];
  openChannel: (channelId: string) => void;
  /** Select the root's card and open its side chat. */
  openThread: (rootId: string) => void;
}): (channelId: string) => void {
  const { getThreadReadAt, threadActivityItems } = useAppShell();

  const pendingRef = React.useRef<{
    channelId: string;
    rootId: string;
  } | null>(null);

  React.useEffect(() => {
    if (
      pendingRef.current &&
      pendingRef.current.channelId !== activeChannelId
    ) {
      pendingRef.current = null;
    }
  }, [activeChannelId]);

  // Complete the jump once the target root is actually on screen. Declared
  // after the shell's selection-reset effect runs for the channel switch,
  // so the reset cannot clobber the opened side chat.
  React.useEffect(() => {
    const pending = pendingRef.current;
    if (!pending || pending.channelId !== activeChannelId) return;
    if (!roots.some((root) => root.id === pending.rootId)) return;
    pendingRef.current = null;
    openThread(pending.rootId);
  }, [activeChannelId, openThread, roots]);

  return React.useCallback(
    (channelId: string) => {
      const mainId = subIndex.parentIdByChildId.get(channelId) ?? channelId;
      const target = findUnreadFamilyTarget({
        familyChannelIds: [
          mainId,
          ...(subIndex.subsByParentId.get(mainId) ?? []).map((sub) => sub.id),
        ],
        unreadChannelIds,
        topLevelUnreadChannelIds,
        threadActivity: threadActivityItems,
        getThreadReadAt,
      });
      if (!target) {
        openChannel(channelId);
        return;
      }
      if (target.channelId === activeChannelId) {
        // Already on the unread tab — just surface the unread thread.
        if (target.rootId && roots.some((root) => root.id === target.rootId)) {
          openThread(target.rootId);
        }
        return;
      }
      openChannel(target.channelId);
      pendingRef.current = target.rootId
        ? { channelId: target.channelId, rootId: target.rootId }
        : null;
    },
    [
      activeChannelId,
      getThreadReadAt,
      openChannel,
      openThread,
      roots,
      subIndex,
      threadActivityItems,
      topLevelUnreadChannelIds,
      unreadChannelIds,
    ],
  );
}
