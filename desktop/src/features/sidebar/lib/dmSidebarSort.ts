import type { Channel } from "@/shared/api/types";

export function sortDmChannelsByLabel(
  channels: Channel[],
  channelLabels: Record<string, string>,
) {
  return [...channels].sort((left, right) => {
    const leftLabel = channelLabels[left.id] ?? left.name;
    const rightLabel = channelLabels[right.id] ?? right.name;
    return (
      leftLabel.localeCompare(rightLabel) || left.id.localeCompare(right.id)
    );
  });
}
