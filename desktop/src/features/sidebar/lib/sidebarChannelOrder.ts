import type { Channel } from "@/shared/api/types";
import type { ChannelSection } from "./channelSectionsStorage";
import {
  sectionSortGroupKey,
  sortChannelsForSidebar,
  type ChannelSortGroupKey,
  type ChannelSortMode,
} from "./channelSortPreference";

/**
 * The sidebar's stream-channel groupings in display form: starred channels,
 * channels bucketed per custom section, and channels assigned to no section.
 * Each grouping is already sorted by its own saved sort preference.
 */
export type SidebarChannelGroups = {
  starred: Channel[];
  bySection: Record<string, Channel[]>;
  unassigned: Channel[];
};

/**
 * Buckets stream channels into the groups the sidebar displays — starred,
 * each live custom section, and unassigned — applying each grouping's own
 * sort preference. Starred channels are excluded from section/unassigned
 * buckets (starring moves a channel, it doesn't duplicate it), and channels
 * assigned to a section that no longer exists fall back to unassigned.
 *
 * This is the single source of truth for sidebar channel ordering: the
 * sidebar renders from it, and keyboard channel navigation flattens it via
 * {@link flattenSidebarChannelGroups}, so the two can't drift.
 */
export function buildSidebarChannelGroups({
  streamChannels,
  starredChannelIds,
  sections,
  assignments,
  sortModeFor,
}: {
  streamChannels: Channel[];
  starredChannelIds: ReadonlySet<string> | undefined;
  sections: ChannelSection[];
  assignments: Record<string, string>;
  sortModeFor: (group: ChannelSortGroupKey) => ChannelSortMode;
}): SidebarChannelGroups {
  const bySection: Record<string, Channel[]> = {};
  const unassigned: Channel[] = [];
  const liveSectionIds = new Set(sections.map((s) => s.id));

  for (const channel of streamChannels) {
    if (starredChannelIds?.has(channel.id)) continue;
    const sectionId = assignments[channel.id];
    if (sectionId && liveSectionIds.has(sectionId)) {
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

  const starred = starredChannelIds?.size
    ? sortChannelsForSidebar(
        streamChannels.filter((channel) => starredChannelIds.has(channel.id)),
        sortModeFor("starred"),
      )
    : [];

  return {
    starred,
    bySection,
    unassigned: sortChannelsForSidebar(unassigned, sortModeFor("channels")),
  };
}

/**
 * Flattens {@link buildSidebarChannelGroups} output into the top-to-bottom
 * order the sidebar displays: starred, then each custom section in section
 * order, then unassigned channels.
 *
 * `sections` must already be in display order (sorted by `order`), matching
 * what `useChannelSections` returns.
 */
export function flattenSidebarChannelGroups(
  groups: SidebarChannelGroups,
  sections: ChannelSection[],
): Channel[] {
  return [
    ...groups.starred,
    ...sections.flatMap((section) => groups.bySection[section.id] ?? []),
    ...groups.unassigned,
  ];
}

/**
 * Returns the id of the channel `direction` steps away from the active one
 * in the flattened sidebar order, skipping muted channels, or null when
 * there is nowhere to go: no active selection, active channel not in the
 * list (e.g. home feed or a DM), or already at the list's end (no
 * wraparound).
 */
export function adjacentSidebarChannelId(
  orderedChannels: Channel[],
  activeChannelId: string | null,
  direction: 1 | -1,
  mutedChannelIds?: ReadonlySet<string>,
): string | null {
  if (!activeChannelId) return null;
  const activeIndex = orderedChannels.findIndex(
    (channel) => channel.id === activeChannelId,
  );
  if (activeIndex === -1) return null;

  for (
    let index = activeIndex + direction;
    index >= 0 && index < orderedChannels.length;
    index += direction
  ) {
    const candidate = orderedChannels[index];
    if (mutedChannelIds?.has(candidate.id)) continue;
    return candidate.id;
  }
  return null;
}
