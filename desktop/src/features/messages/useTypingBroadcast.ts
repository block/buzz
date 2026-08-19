import { useCallback, useRef } from "react";

import { relayClient } from "@/shared/api/relayClient";

const TYPING_SEND_INTERVAL_MS = 3_000;

/**
 * Publishes kind:20002 typing indicators for the current user,
 * throttled to at most once every 3 seconds per channel.
 */
export function useTypingBroadcast(
  channelId: string | null | undefined,
  parentEventId?: string | null,
  rootEventId?: string | null,
) {
  const lastSentRef = useRef(0);
  const lastChannelRef = useRef(channelId);
  const channelIdRef = useRef(channelId);
  const parentEventIdRef = useRef(parentEventId);
  const rootEventIdRef = useRef(rootEventId);
  channelIdRef.current = channelId;
  parentEventIdRef.current = parentEventId;
  rootEventIdRef.current = rootEventId;

  const notifyTyping = useCallback(() => {
    const id = channelIdRef.current;
    if (!id) {
      return;
    }

    if (lastChannelRef.current !== id) {
      lastChannelRef.current = id;
      lastSentRef.current = 0;
    }

    const now = Date.now();
    if (now - lastSentRef.current < TYPING_SEND_INTERVAL_MS) {
      return;
    }

    lastSentRef.current = now;
    relayClient
      .sendTypingIndicator(
        id,
        parentEventIdRef.current ?? null,
        rootEventIdRef.current ?? null,
      )
      .catch(() => {});
  }, []);

  return notifyTyping;
}
