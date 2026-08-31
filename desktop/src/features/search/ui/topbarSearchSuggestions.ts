import type { Channel } from "@/shared/api/types";

export const MAX_RECENT_SEARCH_SUGGESTIONS = 4;

export function getChannelActivityTime(channel: Channel) {
  if (!channel.lastMessageAt) return 0;

  const timestamp = Date.parse(channel.lastMessageAt);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function channelTypeRank(channel: Channel) {
  return channel.channelType === "dm"
    ? 0
    : channel.channelType === "stream"
      ? 1
      : 2;
}

function compareSuggestedChannels(a: Channel, b: Channel) {
  const activityDiff = getChannelActivityTime(b) - getChannelActivityTime(a);
  if (activityDiff !== 0) return activityDiff;

  const rankDiff = channelTypeRank(a) - channelTypeRank(b);
  if (rankDiff !== 0) return rankDiff;

  return a.name.localeCompare(b.name);
}

export function getSuggestedChannels(
  channels: Channel[],
  unreadChannelIds: ReadonlySet<string>,
) {
  const eligibleChannels = channels.filter(
    (channel) =>
      !channel.archivedAt && (channel.isMember || channel.channelType === "dm"),
  );
  const unreadChannels = eligibleChannels
    .filter((channel) => unreadChannelIds.has(channel.id))
    .sort(compareSuggestedChannels);
  const recentChannels = eligibleChannels
    .filter((channel) => !unreadChannelIds.has(channel.id))
    .sort(compareSuggestedChannels)
    .slice(0, MAX_RECENT_SEARCH_SUGGESTIONS);

  return { recentChannels, unreadChannels };
}

export function getSuggestedSearchResults(
  channels: Channel[],
  unreadChannelIds: ReadonlySet<string>,
) {
  const { recentChannels, unreadChannels } = getSuggestedChannels(
    channels,
    unreadChannelIds,
  );
  const toResult = (channel: Channel) => ({
    kind: "channel" as const,
    channel,
  });

  return {
    suggestedResults: recentChannels.map(toResult),
    unreadResults: unreadChannels.map(toResult),
  };
}
