import { useEffect, useEffectEvent, useRef, useState } from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { Channel, RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_DIFF,
  KIND_READ_RECEIPT,
} from "@/shared/constants/kinds";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";

const RECEIPT_SEND_INTERVAL_MS = 5_000;
// Keep receipts briefly so the "Seen" marker survives short re-renders,
// then drop them so the marker disappears when the reader leaves.
const RECEIPT_TTL_MS = 60_000;

/**
 * Publishes kind:20003 ephemeral read receipts for the channel the user is
 * currently viewing, throttled to at most once every 5 seconds, and tracks
 * the latest receipt timestamp from other members.
 *
 * Returns `seenUpTo` — the newest "read at" timestamp (seconds) reported by
 * any other member, or null when nobody else has read recently.
 */
export function useChannelReadReceipts(
  channel: Channel | null,
  currentPubkey: string | undefined,
  latestMessageEvent: RelayEvent | null | undefined,
): number | null {
  const channelId = channel?.id ?? null;
  const channelType = channel?.channelType ?? null;
  const normalizedCurrentPubkey = currentPubkey?.toLowerCase();
  const [seenUpTo, setSeenUpTo] = useState<number | null>(null);
  const latestReceiptAtRef = useRef(0);

  // Broadcast: on channel open and whenever a new message lands, throttled.
  const lastSentRef = useRef(0);
  const lastChannelRef = useRef<string | null>(null);
  const latestMessageId = latestMessageEvent?.id ?? null;
  useEffect(() => {
    if (!channelId || channelType === "forum") {
      return;
    }
    if (lastChannelRef.current !== channelId) {
      lastChannelRef.current = channelId;
      lastSentRef.current = 0;
    }
    if (!latestMessageId) {
      // Still mark the channel open even with no new messages.
    }
    const now = Date.now();
    if (now - lastSentRef.current < RECEIPT_SEND_INTERVAL_MS) {
      return;
    }
    lastSentRef.current = now;
    relayClient.sendReadReceipt(channelId).catch(() => {});
  }, [channelId, channelType, latestMessageId]);

  // Subscribe to other members' receipts.
  const registerReceipt = useEffectEvent((event: RelayEvent) => {
    if (!channelId || event.kind !== KIND_READ_RECEIPT) {
      return;
    }
    if (getChannelIdFromTags(event.tags) !== channelId) {
      return;
    }
    const receiptPubkey = event.pubkey.toLowerCase();
    if (normalizedCurrentPubkey && receiptPubkey === normalizedCurrentPubkey) {
      return;
    }
    if (event.created_at <= latestReceiptAtRef.current) {
      return;
    }
    latestReceiptAtRef.current = event.created_at;
    setSeenUpTo(event.created_at);
  });

  useEffect(() => {
    if (!channelId || channelType === "forum") {
      setSeenUpTo(null);
      return;
    }

    let isDisposed = false;
    let cleanup: (() => Promise<void>) | undefined;

    relayClient
      .subscribeToReadReceipts(channelId, (event) => {
        if (!isDisposed) {
          registerReceipt(event);
        }
      })
      .then((dispose) => {
        if (isDisposed) {
          void dispose();
          return;
        }
        cleanup = dispose;
      })
      .catch((error) => {
        console.error("Failed to subscribe to read receipts", channelId, error);
      });

    const ttlTimer = window.setInterval(() => {
      if (
        latestReceiptAtRef.current > 0 &&
        Date.now() - latestReceiptAtRef.current * 1_000 > RECEIPT_TTL_MS
      ) {
        latestReceiptAtRef.current = 0;
        setSeenUpTo(null);
      }
    }, 15_000);

    return () => {
      isDisposed = true;
      window.clearInterval(ttlTimer);
      if (cleanup) {
        void cleanup();
      }
    };
  }, [channelId, channelType]);

  return seenUpTo;
}

/** True when the event is a stream message that read state can be reported against. */
export function isReceiptableMessageEvent(event: RelayEvent): boolean {
  return (
    event.kind === KIND_STREAM_MESSAGE ||
    event.kind === KIND_STREAM_MESSAGE_DIFF
  );
}
