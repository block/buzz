export type SectionUnreadRollup =
  | { kind: "none" }
  | { kind: "dot" }
  | { kind: "count"; count: number };

export function rollUpSectionUnread(input: {
  channelIds: readonly string[] | undefined;
  highPriorityUnreadChannelIds: ReadonlySet<string> | undefined;
  mutedChannelIds: ReadonlySet<string> | undefined;
  topLevelUnreadChannelIds: ReadonlySet<string> | undefined;
  unreadChannelCounts: ReadonlyMap<string, number> | undefined;
  unreadThreadChannelIds: ReadonlySet<string> | undefined;
}): SectionUnreadRollup;

export function formatSectionUnreadCount(count: number): string;
