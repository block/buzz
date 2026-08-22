/**
 * Utilities for classifying relay connectivity errors.
 *
 * The Rust backend (`desktop/src-tauri/src/relay.rs`) prefixes every
 * "relay unreachable" error message with this literal string so that the
 * frontend can distinguish a transient connectivity failure (e.g. corporate VPN
 * needs reauth, Cloudflare Access 403) from an application-level error.
 *
 * Contract: the Rust layer MUST emit errors starting with exactly this prefix
 * for any condition where the relay host is unreachable at the network or
 * auth layer. Do not change this string without updating relay.rs in lockstep.
 */
const RELAY_UNREACHABLE_PREFIX = "relay unreachable:";

export const RELAY_UNREACHABLE_SHORT = "Can't reach the relay.";
export const RELAY_UNREACHABLE_MESSAGE =
  "Can't reach the relay — check your VPN or network connection.";

/**
 * Returns true when `error` carries the stable Rust-layer prefix indicating
 * the relay is unreachable (network failure, VPN reauth needed, etc.).
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
 * The exact message the backend produces when the relay's membership gate
 * refuses a request.
 *
 * The gate (`crates/buzz-relay/src/api/mod.rs`, `mod relay_members`,
 * `MembershipDecision::Denied`) answers with a **bodyless** 404. With no
 * `message` or `error` field to quote, `classify_relay_error`
 * (`desktop/src-tauri/src/relay.rs`) falls through to its status-only form,
 * producing exactly this string with no `: <detail>` suffix.
 *
 * Every other relay 404 is an `api_error`, which always carries
 * `{"error": "<msg>"}` and therefore renders with a suffix. That is what makes
 * an exact match safe here: a suffixed 404 is a different failure (e.g. no
 * community bound to the host) and must not be reported as a membership
 * problem. `desktop/src/shared/api/tauri.ts` relies on the same equivalence.
 */
const RELAY_MEMBERSHIP_DENIED_404 = "relay returned 404 Not Found";

/**
 * Substrings emitted by relay paths that reject non-members with a message
 * rather than the bodyless 404 above (WebSocket rejections, NIP-29 write
 * refusals). Matched loosely because they arrive embedded in longer text.
 */
const RELAY_MEMBERSHIP_DENIED_SUBSTRINGS = [
  "You must be a relay member",
  "relay_membership_required",
  "restricted: not a relay member",
  "invalid: you are not a relay member",
];

/**
 * Returns true when `error` means "you are not a member of this relay".
 *
 * Onboarding uses this to route to the `MembershipDenied` screen — which
 * offers Retry, Import key, and Change community — instead of dead-ending on
 * raw error text. Missing the bodyless-404 form is what stranded invitees at
 * the profile step (block/buzz#3544): `update_profile` reads before it writes,
 * so the gate rejects the read and onboarding cannot proceed.
 *
 * Accepts `Error` instances and raw strings so callers can pass whatever the
 * Tauri IPC layer hands them without pre-normalizing.
 */
export function isRelayMembershipDeniedError(error: unknown): boolean {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : null;
  if (message === null) return false;

  if (message.trim() === RELAY_MEMBERSHIP_DENIED_404) return true;

  return RELAY_MEMBERSHIP_DENIED_SUBSTRINGS.some((substring) =>
    message.includes(substring),
  );
}
