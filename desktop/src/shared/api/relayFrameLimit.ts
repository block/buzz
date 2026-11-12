/**
 * Utilities for enforcing relay WebSocket frame size limits.
 *
 * The relay may reject oversized frames with a NOTICE message of the form:
 *   "frame too large (N bytes, limit M)"
 * When that happens, the client should NOT reconnect and retry — the same
 * payload will be rejected every time. Instead, surface an OversizedFrameError
 * so callers can propagate it without triggering the reconnect loop.
 */

/** Thrown when a serialised WebSocket frame exceeds the server's size limit. */
export class OversizedFrameError extends Error {
  readonly bytes: number;
  readonly limit: number;

  constructor(bytes: number, limit: number) {
    super(
      `Relay frame too large: ${bytes} bytes exceeds limit of ${limit} bytes`,
    );
    this.name = "OversizedFrameError";
    this.bytes = bytes;
    this.limit = limit;
  }
}

/** Type-guard for OversizedFrameError. */
export function isOversizedFrameError(
  error: unknown,
): error is OversizedFrameError {
  return error instanceof OversizedFrameError;
}

/**
 * Parse a relay NOTICE that signals a frame-size rejection.
 *
 * Matches: `frame too large (N bytes, limit M)`
 * Returns `{ bytes, limit }` on match, or `null` otherwise.
 */
export function parseFrameTooLargeNotice(
  notice: string,
): { bytes: number; limit: number } | null {
  const match = /frame too large \((\d+) bytes, limit (\d+)\)/.exec(notice);
  if (!match) return null;
  return { bytes: Number(match[1]), limit: Number(match[2]) };
}

/**
 * Compute the UTF-8 byte length of a serialised WebSocket payload.
 *
 * This mirrors the byte count the relay sees on the wire so the client can
 * pre-flight frames before sending, avoiding a round-trip rejection.
 */
export function relayFrameByteLength(payload: unknown[]): number {
  return new TextEncoder().encode(JSON.stringify(payload)).byteLength;
}

/**
 * Assert that the payload's wire size is within `maxBytes`.
 *
 * @throws {OversizedFrameError} when the frame would exceed the limit.
 */
export function assertWithinFrameLimit(
  payload: unknown[],
  maxBytes: number,
): void {
  const bytes = relayFrameByteLength(payload);
  if (bytes > maxBytes) {
    throw new OversizedFrameError(bytes, maxBytes);
  }
}
