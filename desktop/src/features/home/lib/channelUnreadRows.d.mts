import type { Channel } from "@/shared/api/types";

export type ChannelUnreadRow = {
  key: string;
  kind: "channel";
  channelId: string;
  channel: Channel;
  unreadCount: number;
  sortAt: number;
};

export function buildChannelUnreadRows(input: {
  channels: readonly Channel[] | undefined;
  latestUnreadActivityByChannelId: ReadonlyMap<string, number> | undefined;
  mutedChannelIds: ReadonlySet<string> | undefined;
  topLevelUnreadChannelIds: ReadonlySet<string> | undefined;
  unreadChannelCounts: ReadonlyMap<string, number> | undefined;
  unreadThreadChannelIds: ReadonlySet<string> | undefined;
}): ChannelUnreadRow[];

export function withoutDuplicatedChannels(
  channelRows: readonly ChannelUnreadRow[],
  occupiedChannelIds: ReadonlySet<string> | undefined,
): ChannelUnreadRow[];
