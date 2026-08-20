import type { Channel } from "@/shared/api/types";

/**
 * Channels whose name starts with this prefix are discovered as workstream
 * board entries. Discovery is limited to visible channels the current user
 * has joined; there is no creator/ownership filter.
 */
export const WORKSTREAM_CHANNEL_PREFIX = "loganj-ws-";

export function filterWorkstreamChannels(
  channels: readonly Channel[],
): Channel[] {
  return channels.filter(
    (channel) =>
      channel.isMember &&
      channel.archivedAt === null &&
      channel.name.startsWith(WORKSTREAM_CHANNEL_PREFIX),
  );
}
