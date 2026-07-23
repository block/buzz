import type { Channel } from "@/shared/api/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import {
  buildMainTimelineEntries,
  type MainTimelineEntry,
} from "@/features/messages/lib/threadPanel";
import { shouldFlattenChannelTimeline } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";

/** Main-timeline entries with private/DM reply flatten applied when appropriate. */
export function buildChannelMainTimelineEntries(
  channel: Pick<Channel, "channelType" | "visibility"> | null | undefined,
  messages: TimelineMessage[],
  threadSummaries: ReadonlyMap<string, ChannelWindowThreadSummary>,
  profiles?: UserProfileLookup,
): { entries: MainTimelineEntry[]; flattenReplies: boolean } {
  const flattenReplies = shouldFlattenChannelTimeline(channel);
  return {
    flattenReplies,
    entries: buildMainTimelineEntries(
      messages,
      new Set(),
      threadSummaries,
      profiles,
      {
        flattenReplies,
      },
    ),
  };
}
