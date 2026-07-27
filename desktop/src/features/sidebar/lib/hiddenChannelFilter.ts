/**
 * Drop "Mute and hide" channels (NIP-CN level "mute" set explicitly) from a
 * sidebar list, with Slack's two escape hatches: the channel the viewer is
 * looking at stays put, and a channel holding a mention-tier unread resurfaces
 * (rendered in the existing muted styling by the row itself).
 *
 * Pure: the caller supplies the resolved hidden set and the mention-tier set.
 */
export function filterHiddenChannels<T extends { id: string }>(
  channels: readonly T[],
  options: {
    hiddenChannelIds?: ReadonlySet<string>;
    activeChannelId?: string | null;
    mentionUnreadChannelIds?: ReadonlySet<string>;
  },
): readonly T[] {
  const hidden = options.hiddenChannelIds;
  if (!hidden || hidden.size === 0) return channels;

  return channels.filter(
    (channel) =>
      !hidden.has(channel.id) ||
      channel.id === options.activeChannelId ||
      Boolean(options.mentionUnreadChannelIds?.has(channel.id)),
  );
}
