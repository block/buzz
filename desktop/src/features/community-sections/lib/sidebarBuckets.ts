import type { Channel } from "@/shared/api/types";

import type { CommunitySection } from "./types";

/**
 * Channels belonging to any subscribed community section.
 * Used to keep those channels out of personal sections / Channels group.
 */
export function communitySectionChannelIds(
  sections: ReadonlyArray<CommunitySection>,
): Set<string> {
  const ids = new Set<string>();
  for (const section of sections) {
    for (const channelId of section.channelIds) {
      ids.add(channelId);
    }
  }
  return ids;
}

export function channelsForCommunitySection(
  section: CommunitySection,
  channelsById: ReadonlyMap<string, Channel>,
  starredChannelIds?: ReadonlySet<string> | null,
): Channel[] {
  const result: Channel[] = [];
  for (const channelId of section.channelIds) {
    if (starredChannelIds?.has(channelId)) continue;
    const channel = channelsById.get(channelId);
    if (channel) result.push(channel);
  }
  return result;
}
