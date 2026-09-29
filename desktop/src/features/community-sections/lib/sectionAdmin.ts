import { looksLikeHuddleBackingChannel } from "@/app/huddleChannelVisibility";
import type { Channel } from "@/shared/api/types";

import type { CommunitySection } from "./types";

/** Stream channels eligible for community-section pickers (no huddle backing). */
export function isCommunitySectionPickerChannel(
  channel: Pick<Channel, "channelType" | "name" | "ttlSeconds" | "id">,
): boolean {
  if (channel.channelType !== "stream") return false;
  return !looksLikeHuddleBackingChannel(channel);
}

/** Channel ids already listed in any section other than `exceptSectionId`. */
export function channelIdsClaimedByOtherSections(
  sections: ReadonlyArray<CommunitySection>,
  exceptSectionId: string | null,
): Set<string> {
  const claimed = new Set<string>();
  for (const section of sections) {
    if (exceptSectionId && section.id === exceptSectionId) continue;
    for (const channelId of section.channelIds) {
      claimed.add(channelId);
    }
  }
  return claimed;
}

/**
 * Keep each channel in at most one section (first wins by catalog order).
 * Used when publishing so exclusivity survives older catalogs.
 */
export function ensureExclusiveChannelMembership(
  sections: ReadonlyArray<CommunitySection>,
): CommunitySection[] {
  const claimed = new Set<string>();
  return sections.map((section) => {
    const channelIds: string[] = [];
    for (const channelId of section.channelIds) {
      if (claimed.has(channelId)) continue;
      claimed.add(channelId);
      channelIds.push(channelId);
    }
    return channelIds.length === section.channelIds.length
      ? section
      : { ...section, channelIds };
  });
}

/** Reorder sections and rewrite ascending `order` to match. */
export function reorderCommunitySections(
  sections: ReadonlyArray<CommunitySection>,
  orderedIds: ReadonlyArray<string>,
): CommunitySection[] {
  const byId = new Map(sections.map((section) => [section.id, section]));
  const next: CommunitySection[] = [];
  const seen = new Set<string>();
  for (const id of orderedIds) {
    const section = byId.get(id);
    if (!section || seen.has(id)) continue;
    seen.add(id);
    next.push({ ...section, order: next.length });
  }
  for (const section of sections) {
    if (seen.has(section.id)) continue;
    next.push({ ...section, order: next.length });
  }
  return next;
}
