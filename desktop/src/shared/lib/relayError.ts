/**
 * Utilities for classifying relay connectivity errors.
 *
 * The Rust backend (`desktop/src-tauri/src/relay.rs`) prefixes every
 * "relay unreachable" error message with this literal string so that the
 * frontend can distinguish a transient connectivity failure from an
 * application-level error.
 *
 * Contract: the Rust layer MUST emit errors starting with exactly this prefix
 * for any condition where the relay host is unreachable at the network or
 * auth layer. Do not change this string without updating relay.rs in lockstep.
 */
const RELAY_UNREACHABLE_PREFIX = "relay unreachable:";

export const RELAY_UNREACHABLE_SHORT = "Can't reach the relay.";
export const RELAY_UNREACHABLE_MESSAGE =
  "Can't reach the relay — check your network connection.";

/**
 * Returns true when `error` carries the stable Rust-layer prefix indicating
 * the relay is unreachable (network failure, transport reauth needed, etc.).
 *
 * Accepts both `Error` instances and raw strings so callers can pass whatever
 * the Tauri IPC or WebSocket layer hands them without pre-normalizing.
 */
export function isRelayUnreachableError(error: unknown): boolean {
  if (error instanceof Error) {
    return error.message.startsWith(RELAY_UNREACHABLE_PREFIX);
  }
  if (typeof error === "string") {
    return error.startsWith(RELAY_UNREACHABLE_PREFIX);
  }
  return false;
}

/**
 * Returns a human-readable detail string for an error.
 *
 * When the error is classified as a relay-unreachable error, strips the
 * prefix and trims whitespace so the UI sees only the Rust-authored detail
 * (e.g. "connection refused" or "403 Forbidden").
 *
 * Falls back to a generic connectivity message for anything unclassified.
 */
export function relayErrorDetail(error: unknown): string {
  if (isRelayUnreachableError(error)) {
    const message = error instanceof Error ? error.message : (error as string);
    const detail = message.slice(RELAY_UNREACHABLE_PREFIX.length).trim();
    return detail || RELAY_UNREACHABLE_MESSAGE;
  }
  return RELAY_UNREACHABLE_MESSAGE;
}
