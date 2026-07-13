import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Returns the pubkey to use as `configNudgeAuthorPubkey` for a given message,
 * or `undefined` when the config-nudge card path should be disabled.
 *
 * The card is enabled ONLY when:
 *   1. `message.kind === KIND_STREAM_MESSAGE` — restricts to the setup-listener
 *      wire format.
 *   2. `message.signerPubkey` is set and passes `isKnownAgentPubkey` —
 *      authenticates against the raw event signer (NOT `message.pubkey`,
 *      which is the tag-attributed display author and can be spoofed via
 *      `actor`/`p` tags). The caller's predicate combines the workspace-wide
 *      known-agent baseline (`useKnownAgentPubkeys`) with any surface-local
 *      signals such as the signer profile's `isAgent` flag.
 *
 * Extracting this predicate as a pure helper lets tests exercise the exact
 * signer-vs-attributed-author distinction with a real `TimelineMessage` from
 * `formatTimelineMessages`, without a full React render harness.
 */
export function getConfigNudgeAuthorPubkey(
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
