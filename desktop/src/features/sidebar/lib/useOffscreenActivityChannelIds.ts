import * as React from "react";

type OffscreenActivityChannelIds = {
  messageChannelIds: ReadonlySet<string>;
  channelIds: ReadonlySet<string>;
};

export function getOffscreenActivityChannelIds({
  activeWorkingByChannelId,
  previewActivityChannelIds,
  unreadChannelIds,
}: {
  activeWorkingByChannelId: ReadonlyMap<string, unknown>;
  previewActivityChannelIds: ReadonlySet<string>;
  unreadChannelIds: ReadonlySet<string>;
}): OffscreenActivityChannelIds {
  // Every unread row must remain navigable, including top-level stream and
  // forum unreads that do not have thread-preview activity.
  const messageChannelIds = new Set([
    ...unreadChannelIds,
    ...previewActivityChannelIds,
  ]);

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
  previewActivityChannelIds: ReadonlySet<string>;
  unreadChannelIds: ReadonlySet<string>;
}) {
  const {
    activeWorkingByChannelId,
    previewActivityChannelIds,
    unreadChannelIds,
  } = args;

  return React.useMemo(
    () =>
      getOffscreenActivityChannelIds({
        activeWorkingByChannelId,
        previewActivityChannelIds,
        unreadChannelIds,
      }),
    [activeWorkingByChannelId, previewActivityChannelIds, unreadChannelIds],
  );
}
