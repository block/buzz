import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";

type SendToChannel = (
  message: TimelineMessage,
  threadRoot: TimelineMessage,
  channelId: string,
) => Promise<void>;

export function useStableSendToChannel(
  channelId: string | null,
  threadHead: TimelineMessage | null,
  onSendToChannel?: SendToChannel,
): ((message: TimelineMessage) => Promise<void>) | undefined {
  const contextRef = React.useRef({ channelId, onSendToChannel, threadHead });
  React.useLayoutEffect(() => {
    contextRef.current = { channelId, onSendToChannel, threadHead };
  }, [channelId, onSendToChannel, threadHead]);
  const sendToChannel = React.useCallback((message: TimelineMessage) => {
    const context = contextRef.current;
    if (!context.onSendToChannel || !context.threadHead || !context.channelId) {
      return Promise.resolve();
    }
    return context.onSendToChannel(
      message,
      context.threadHead,
      context.channelId,
    );
  }, []);
  return onSendToChannel && channelId ? sendToChannel : undefined;
}
