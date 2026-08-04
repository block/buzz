import * as React from "react";

import type { Channel } from "@/shared/api/types";

type OffscreenActivityChannelIds = {
  messageChannelIds: ReadonlySet<string>;
  channelIds: ReadonlySet<string>;
};

export function getOffscreenActivityChannelIds({
  activeWorkingByChannelId,
  channels,
  previewActivityChannelIds,
  unreadChannelIds,
}: {
  activeWorkingByChannelId: ReadonlyMap<string, unknown>;
  channels: readonly Channel[];
  previewActivityChannelIds: ReadonlySet<string>;
  unreadChannelIds: ReadonlySet<string>;
}): OffscreenActivityChannelIds {
  const messageChannelIds = new Set(previewActivityChannelIds);

  // Direct messages use their own unread indicator rather than the channel
  // preview dot, but should remain discoverable when their row is offscreen.
  for (const channel of channels) {
    if (channel.channelType === "dm" && unreadChannelIds.has(channel.id)) {
      messageChannelIds.add(channel.id);
    }
  }

  return {
    messageChannelIds,
    channelIds: new Set([
      ...messageChannelIds,
      ...activeWorkingByChannelId.keys(),
    ]),
  };
}

export function useOffscreenActivityChannelIds(args: {
  activeWorkingByChannelId: ReadonlyMap<string, unknown>;
  channels: readonly Channel[];
  previewActivityChannelIds: ReadonlySet<string>;
  unreadChannelIds: ReadonlySet<string>;
}) {
  const {
    activeWorkingByChannelId,
    channels,
    previewActivityChannelIds,
    unreadChannelIds,
  } = args;

  return React.useMemo(
    () =>
      getOffscreenActivityChannelIds({
        activeWorkingByChannelId,
        channels,
        previewActivityChannelIds,
        unreadChannelIds,
      }),
    [
      activeWorkingByChannelId,
      channels,
      previewActivityChannelIds,
      unreadChannelIds,
    ],
  );
}
