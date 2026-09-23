import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import type { TimelineMessage } from "@/features/messages/types";
import { computePermissionRequest } from "@/shared/lib/computePermissionRequest";
import type { PermissionRequestPayload } from "@/shared/lib/permissionRequest";

/**
 * Returns the agent pubkey to use for the `PermissionRequestCard` for a given
 * message, or `undefined` when the permission-card path should be disabled.
 *
 * The card is enabled ONLY when:
 *   1. `message.kind === KIND_STREAM_MESSAGE` — restricts to the setup-listener
 *      wire format (kind:9).
 *   2. `message.signerPubkey` is set and passes `isKnownAgentPubkey` —
 *      authenticates against the raw event signer (NOT `message.pubkey`,
 *      which may be a relay-delegated display author).
 *
 * Mirrors `getConfigNudgeAuthorPubkey` — same signer-vs-delegated-author
 * distinction, same test-friendly pure-function shape.
 */
export function getPermissionRequestAgentPubkey(
  message: Pick<TimelineMessage, "kind" | "signerPubkey">,
  isKnownAgentPubkey: (pubkey: string) => boolean,
): string | undefined {
  if (
    message.kind === KIND_STREAM_MESSAGE &&
    message.signerPubkey &&
    isKnownAgentPubkey(message.signerPubkey)
  ) {
    return message.signerPubkey;
  }
  return undefined;
}

/**
 * Pre-computed permission-request result — the single trusted payload and the
 * authenticated agent pubkey. Returned by `selectPermissionRequest` when the
 * card will render; `null` when it will not.
 */
export type PermissionRequestSelection = {
  agentPubkey: string;
  request: PermissionRequestPayload;
};

/**
 * Computes the permission-request card payload ONCE, incorporating all render
 * eligibility checks — including `channelId` and `message.isAgent` — so the
 * result can be used for BOTH prose suppression in `MessageRow` AND as the
 * pre-computed input to `PermissionRequestCardBlock`.
 *
 * Returns non-null iff a card will render, by construction:
 *   - `channelId` is truthy (falsy channelId → no card → no prose suppression)
 *   - `message.isAgent` is true
 *   - signer is a known agent (`getPermissionRequestAgentPubkey` succeeds)
 *   - `computePermissionRequest` returns a non-null payload
 *
 * This is the single source of truth for the prose-suppression decision in
 * `MessageRow`. Passing this result to `PermissionRequestCardBlock` closes
 * the double-computation gap and ensures prose is suppressed iff the card
 * renders — by construction, not by approximation.
 *
 * Mirrors `selectProseOrPermission` — the card's prose-suppression contract.
 */
export function selectPermissionRequest(
  message: Pick<
    TimelineMessage,
    | "kind"
    | "isAgent"
    | "signerPubkey"
    | "body"
    | "editSignerPubkey"
    | "id"
    | "preEditBody"
  >,
  isKnownAgentPubkey: (pubkey: string) => boolean,
  channelId: string | null | undefined,
): PermissionRequestSelection | null {
  if (!channelId || !message.isAgent) return null;
  const agentPubkey = getPermissionRequestAgentPubkey(
    message,
    isKnownAgentPubkey,
  );
  const request = computePermissionRequest(
    message.body,
    true,
    agentPubkey,
    message.signerPubkey,
    message.editSignerPubkey,
    message.id,
    message.preEditBody,
  );
  if (request === null || !agentPubkey) return null;
  return { agentPubkey, request };
}

/**
 * Returns `true` only when `selectPermissionRequest` returns a non-null
 * selection — i.e., when a card will render.
 *
 * Kept as a thin delegate over `selectPermissionRequest` for callers that only
 * need a boolean (e.g. MessageRow's prose-suppression guard). The prose guard
 * and the card block both derive from the same `selectPermissionRequest` call,
 * so prose is suppressed iff the card renders.
 *
 * @deprecated Use `selectPermissionRequest` directly when you also need the
 * computed agentPubkey/request to pass to the block.
 */
export function hasPermissionRequestCard(
  message: Pick<
    TimelineMessage,
    | "kind"
    | "isAgent"
    | "signerPubkey"
    | "body"
    | "editSignerPubkey"
    | "id"
    | "preEditBody"
  >,
  isKnownAgentPubkey: (pubkey: string) => boolean,
): boolean {
  // Note: channelId is not available here; MessageRow should use
  // selectPermissionRequest directly and pass channelId.
  const agentPubkey = getPermissionRequestAgentPubkey(
    message,
    isKnownAgentPubkey,
  );
  return (
    computePermissionRequest(
      message.body,
      true,
      agentPubkey,
      message.signerPubkey,
      message.editSignerPubkey,
      message.id,
      message.preEditBody,
    ) !== null
  );
}
