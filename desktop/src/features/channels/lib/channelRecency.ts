import type { QueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import type { Channel } from "@/shared/api/types";

function parseTimestamp(value: number | string | null | undefined) {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value * 1_000 : null;
  }

  if (!value) {
    return null;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? null : timestamp;
}

export function applyChannelLastMessageAt(
  current: Channel[] | undefined,
  channelId: string,
  lastMessageAt: number | string | null | undefined,
): Channel[] | undefined {
  if (!current) {
    return current;
  }

  const candidateTimestamp = parseTimestamp(lastMessageAt);
  if (candidateTimestamp === null) {
    return current;
  }

  let didUpdate = false;
  const normalizedLastMessageAt = new Date(candidateTimestamp).toISOString();
  const nextChannels = current.map((channel) => {
    if (channel.id !== channelId) {
      return channel;
    }

    const currentTimestamp = parseTimestamp(channel.lastMessageAt);
    if (currentTimestamp !== null && candidateTimestamp <= currentTimestamp) {
      return channel;
    }

    didUpdate = true;
    return {
      ...channel,
      lastMessageAt: normalizedLastMessageAt,
    };
  });

  return didUpdate ? nextChannels : current;
}

export function updateChannelLastMessageAt(
  queryClient: QueryClient,
  channelId: string,
  lastMessageAt: number | string | null | undefined,
) {
  queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
    applyChannelLastMessageAt(current, channelId, lastMessageAt),
  );
}
