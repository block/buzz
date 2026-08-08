import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import type { TimelineMessage } from "@/features/messages/types";

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
