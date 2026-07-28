import * as React from "react";

import {
  sectionSortGroupKey,
  sortChannelsForSidebar,
  type ChannelSortGroupKey,
  type ChannelSortMode,
} from "@/features/sidebar/lib/channelSortPreference";
import { filterHiddenChannels } from "@/features/sidebar/lib/hiddenChannelFilter";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import type { Channel } from "@/shared/api/types";

export type SidebarChannelGroups = {
  /** Stream channels, minus NIP-CN hidden ones. */
  streamChannels: Channel[];
  /** Stream channels bucketed by custom section, each sorted by its own mode. */
  sectionBuckets: {
    bySection: Record<string, Channel[]>;
    unassigned: Channel[];
  };
  starredChannels: Channel[];
  forumChannels: Channel[];
};

/**
 * Derives the sidebar's channel groupings: hide filtering (NIP-CN "Mute and
 * hide"), section bucketing, and per-grouping sort. Pure derivation kept out of
 * the sidebar component so `AppSidebar` only renders.
 */
export function useSidebarChannelGroups({
  activeChannelId,
  channelAssignments,
  channels,
  channelSections,
  hiddenChannelIds,
  mentionUnreadChannelIds,
  sortModeFor,
  starredChannelIds,
}: {
  activeChannelId: string | null;
  channelAssignments: Record<string, string>;
  channels: Channel[];
  channelSections: ChannelSection[];
  hiddenChannelIds?: ReadonlySet<string>;
  mentionUnreadChannelIds?: ReadonlySet<string>;
  sortModeFor: (group: ChannelSortGroupKey) => ChannelSortMode;
  starredChannelIds?: ReadonlySet<string>;
}): SidebarChannelGroups {
  // Hidden channels leave the sidebar lists but stay reachable from search and
  // the quick switcher, which read the unfiltered channel set.
  const visibleChannels = React.useMemo(
    () =>
      filterHiddenChannels(channels, {
        activeChannelId,
        hiddenChannelIds,
        mentionUnreadChannelIds,
      }),
    [channels, activeChannelId, hiddenChannelIds, mentionUnreadChannelIds],
  );

  const streamChannels = React.useMemo(
    () => visibleChannels.filter((channel) => channel.channelType === "stream"),
    [visibleChannels],
  );

  const sectionBuckets = React.useMemo(() => {
    const bySection: Record<string, Channel[]> = {};
    const unassigned: Channel[] = [];
    const sectionIds = new Set(channelSections.map((s) => s.id));

    for (const channel of streamChannels) {
      if (starredChannelIds?.has(channel.id)) continue;
      const sectionId = channelAssignments[channel.id];
      if (sectionId && sectionIds.has(sectionId)) {
        if (!bySection[sectionId]) {
          bySection[sectionId] = [];
        }
        bySection[sectionId].push(channel);
      } else {
        unassigned.push(channel);
      }
    }
    // Apply each grouping's own sort preference; section membership itself
    // is untouched.
    for (const sectionId of Object.keys(bySection)) {
      bySection[sectionId] = sortChannelsForSidebar(
        bySection[sectionId],
        sortModeFor(sectionSortGroupKey(sectionId)),
      );
    }
    return {
      bySection,
      unassigned: sortChannelsForSidebar(unassigned, sortModeFor("channels")),
    };
  }, [
    streamChannels,
    channelSections,
    channelAssignments,
    starredChannelIds,
    sortModeFor,
  ]);

  const starredChannels = React.useMemo(() => {
    if (!starredChannelIds || starredChannelIds.size === 0) return [];
    return sortChannelsForSidebar(
      streamChannels.filter((channel) => starredChannelIds.has(channel.id)),
      sortModeFor("starred"),
    );
  }, [streamChannels, starredChannelIds, sortModeFor]);

  const forumChannels = React.useMemo(
    () =>
      sortChannelsForSidebar(
        visibleChannels.filter((channel) => channel.channelType === "forum"),
        sortModeFor("forums"),
      ),
    [visibleChannels, sortModeFor],
  );

  return { streamChannels, sectionBuckets, starredChannels, forumChannels };
}
