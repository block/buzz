/**
 * Low-level WebSocket send helpers for the relay client.
 *
 * Extracted from RelayClient so relayClientSession.ts stays within its line
 * budget. This module owns:
 *  - frame serialisation and byte-limit enforcement
 *  - the reconnect-retry wrapper (with OversizedFrameError short-circuit)
 *  - frame-size limit bookkeeping from relay NOTICE messages
 */

import { invoke } from "@tauri-apps/api/core";

import {
  assertWithinFrameLimit,
  isOversizedFrameError,
  parseFrameTooLargeNotice,
} from "@/shared/api/relayFrameLimit";

/**
 * Serialise `payload` as JSON and send it as a WebSocket text frame.
 *
 * @param wsId - Active socket ID (null → throws "not connected").
 * @param payload - Array to serialise. Must be non-empty.
 * @param maxFrameBytes - When non-null, asserts the serialised size is within
 *   the limit before sending; throws {@link OversizedFrameError} otherwise.
 */
export async function sendRelayTextFrame(
  wsId: number | null,
  payload: unknown[],
  maxFrameBytes: number | null,
): Promise<void> {
  if (wsId === null) {
    throw new Error("Relay socket is not connected.");
  }
  if (maxFrameBytes !== null) {
    assertWithinFrameLimit(payload, maxFrameBytes);
  }
  await invoke("plugin:websocket|send", {
    id: wsId,
    message: {
      type: "Text",
      data: JSON.stringify(payload),
    },
  });
}

/**
 * Parse a relay NOTICE for a frame-size limit update.
 *
 * Returns the new limit (taken from the NOTICE) when the notice matches
 * *and* the reported limit is smaller than `currentMax` (or `currentMax` is
 * null), indicating the client should tighten its frame budget. Returns
 * `null` when the notice is unrelated or the limit has not decreased.
 */
export function noteFrameTooLargeLimit(
  notice: string,
  currentMax: number | null,
): number | null {
  const parsed = parseFrameTooLargeNotice(notice);
  if (!parsed) return null;
  const { limit } = parsed;
  if (currentMax !== null && currentMax <= limit) return null;
  return limit;
}

/**
 * Send a relay frame, retrying once after a reconnect on transient failures.
 *
 * {@link OversizedFrameError} is **never** retried — the same payload will be
 * rejected every time, so retrying would only trigger another reconnect storm.
 *
 * @param options.payload - Frame to send.
 * @param options.fallbackMessage - Error message used when the caught error
 *   cannot be normalised.
 * @param options.sendRaw - Sends the frame on the current socket.
 * @param options.recoverFromSocketFailure - Called on socket errors; resets
 *   the connection and returns a normalised Error.
 * @param options.ensureConnected - Awaits (re)connection before the retry.
 */
export async function sendRelayTextFrameWithReconnectRetry({
  payload,
  fallbackMessage,
  sendRaw,
  recoverFromSocketFailure,
  ensureConnected,
}: {
  payload: unknown[];
  fallbackMessage: string;
  sendRaw: (payload: unknown[]) => Promise<void>;
  recoverFromSocketFailure: (error: unknown, message: string) => Error;
  ensureConnected: () => Promise<void>;
}): Promise<void> {
  try {
    await sendRaw(payload);
  } catch (error) {
    // Never retry oversized frames — the same payload will always be rejected.
    if (isOversizedFrameError(error)) {
      throw error;
    }
    const normalizedError = recoverFromSocketFailure(error, fallbackMessage);
    try {
      await ensureConnected();
      await sendRaw(payload);
    } catch (retryError) {
      throw recoverFromSocketFailure(retryError, normalizedError.message);
    }
  }
}
