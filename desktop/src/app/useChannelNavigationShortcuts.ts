import * as React from "react";

import { readChannelSectionsStore } from "@/features/sidebar/lib/channelSectionsStorage";
import {
  readChannelSortStore,
  sortModeForGroup,
  type ChannelSortGroupKey,
} from "@/features/sidebar/lib/channelSortPreference";
import {
  adjacentSidebarChannelId,
  buildSidebarChannelGroups,
  flattenSidebarChannelGroups,
} from "@/features/sidebar/lib/sidebarChannelOrder";
import type { Channel } from "@/shared/api/types";
import { isMacPlatform } from "@/shared/lib/platform";

/**
 * Next/previous channel keyboard navigation: ⌥↓ / ⌥↑ on macOS and
 * Ctrl+Alt+↓ / Ctrl+Alt+↑ on Windows/Linux move the active channel selection
 * down/up the sidebar's stream-channel list (starred → custom sections →
 * unassigned, each in its own saved sort order). Windows/Linux includes Ctrl
 * because plain Alt+arrows are taken by back/forward navigation
 * (useBackForwardControls).
 *
 * Scope matches the sidebar's Channels area: the active community's stream
 * channels only — forums and DMs are not part of the cycle. Muted channels
 * are skipped, the selection stops at both ends (no wraparound), and the
 * shortcut is a no-op when no channel is selected (e.g. home feed) or the
 * selected conversation isn't in the list.
 *
 * Section membership and per-group sort preferences are read from their
 * relay-scoped localStorage stores at keypress time — the same stores the
 * sidebar hooks keep in sync (locally and from remote NIP-78 blobs) — so the
 * traversal order can't drift from what the sidebar displays without
 * duplicating the sidebar's relay subscriptions here.
 */
export function useChannelNavigationShortcuts({
  channels,
  currentPubkey,
  mutedChannelIds,
  onSelectChannel,
  relayUrl,
  selectedChannelId,
  selectedView,
  starredChannelIds,
}: {
  channels: Channel[];
  currentPubkey: string | undefined;
  mutedChannelIds: ReadonlySet<string>;
  onSelectChannel: (channelId: string) => void;
  relayUrl: string | undefined;
  selectedChannelId: string | null;
  selectedView: string;
  starredChannelIds: ReadonlySet<string> | undefined;
}) {
  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      if (event.defaultPrevented) return;

      const matchesCombo = isMacPlatform()
        ? event.altKey && !event.metaKey && !event.ctrlKey && !event.shiftKey
        : event.ctrlKey && event.altKey && !event.metaKey && !event.shiftKey;
      if (!matchesCombo) return;

      if (selectedView !== "channel" || !selectedChannelId) return;

      const streamChannels = channels.filter(
        (channel) => channel.channelType === "stream",
      );
      const sectionsStore = currentPubkey
        ? readChannelSectionsStore(currentPubkey, relayUrl)
        : null;
      const sortStore = currentPubkey
        ? readChannelSortStore(currentPubkey, relayUrl)
        : null;
      const sections = (sectionsStore?.sections ?? [])
        .slice()
        .sort((a, b) => a.order - b.order);
      const sortModeFor = (group: ChannelSortGroupKey) =>
        sortStore ? sortModeForGroup(sortStore, group) : ("alpha" as const);

      const ordered = flattenSidebarChannelGroups(
        buildSidebarChannelGroups({
          streamChannels,
          starredChannelIds,
          sections,
          assignments: sectionsStore?.assignments ?? {},
          sortModeFor,
        }),
        sections,
      );

      // Only act when the active conversation is actually in the sidebar's
      // stream list — for forums/DMs the combo keeps its default behavior.
      if (!ordered.some((channel) => channel.id === selectedChannelId)) return;

      // The shortcut owns this combo whenever a stream channel is active,
      // including at the list's ends — a boundary no-op shouldn't fall
      // through to scrolling or text-caret movement.
      event.preventDefault();

      const nextChannelId = adjacentSidebarChannelId(
        ordered,
        selectedChannelId,
        event.key === "ArrowDown" ? 1 : -1,
        mutedChannelIds,
      );
      if (nextChannelId) {
        onSelectChannel(nextChannelId);
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    channels,
    currentPubkey,
    mutedChannelIds,
    onSelectChannel,
    relayUrl,
    selectedChannelId,
    selectedView,
    starredChannelIds,
  ]);
}
