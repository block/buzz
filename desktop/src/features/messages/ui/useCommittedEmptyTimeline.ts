import * as React from "react";

/** Track only empty timelines that React actually committed for this channel. */
export function useCommittedEmptyTimeline({
  channelId,
  deferredCount,
  hasPersistentIntro,
  isLoading,
  liveCount,
}: {
  channelId: string | null;
  deferredCount: number;
  hasPersistentIntro: boolean;
  isLoading: boolean;
  liveCount: number;
}) {
  const committedRef = React.useRef({
    channelId: null as string | null,
    hasSettledEmpty: false,
  });
  const preserveSettledEmptyIntro =
    hasPersistentIntro &&
    committedRef.current.channelId === channelId &&
    committedRef.current.hasSettledEmpty;

  React.useLayoutEffect(() => {
    if (isLoading) return;
    committedRef.current = {
      channelId,
      hasSettledEmpty: liveCount === 0 && deferredCount === 0,
    };
  }, [channelId, deferredCount, isLoading, liveCount]);

  return preserveSettledEmptyIntro;
}
