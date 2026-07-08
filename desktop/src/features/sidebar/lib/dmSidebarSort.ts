import type { Channel } from "@/shared/api/types";
import type { ChannelSortMode } from "@/features/sidebar/lib/channelSortPreference";

function compareDmChannelsByLabel(
  left: Channel,
  right: Channel,
  channelLabels: Record<string, string>,
): number {
  const leftLabel = channelLabels[left.id] ?? left.name;
  const rightLabel = channelLabels[right.id] ?? right.name;
  return leftLabel.localeCompare(rightLabel) || left.id.localeCompare(right.id);
}

export function sortDmChannelsByLabel(
  channels: Channel[],
  channelLabels: Record<string, string>,
) {
  return [...channels].sort((left, right) =>
    compareDmChannelsByLabel(left, right, channelLabels),
  );
}

function dmRecencyMs(channel: Channel): number | null {
  if (!channel.lastMessageAt) return null;
  const ms = Date.parse(channel.lastMessageAt);
  return Number.isFinite(ms) ? ms : null;
}

/**
 * Sorts sidebar DMs by the selected mode. `alpha` keeps the existing
 * resolved-label ordering; `recent` orders by last message time, newest
 * first, with quiet DMs sinking to the bottom in label order.
 */
export function sortDmChannelsForSidebar(
  channels: Channel[],
  channelLabels: Record<string, string>,
  mode: ChannelSortMode,
) {
  if (mode === "alpha") {
    return sortDmChannelsByLabel(channels, channelLabels);
  }
  return [...channels].sort((left, right) => {
    const leftMs = dmRecencyMs(left);
    const rightMs = dmRecencyMs(right);
    if (leftMs !== null && rightMs !== null && leftMs !== rightMs) {
      return rightMs - leftMs;
    }
    if (leftMs !== null && rightMs === null) return -1;
    if (leftMs === null && rightMs !== null) return 1;
    return compareDmChannelsByLabel(left, right, channelLabels);
  });
}
