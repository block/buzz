import type { TimelineMessage } from "@/features/messages/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * The minimal shape of the message a reply is anchored to (its parent /
 * reply target). Kept as a structural subset of {@link TimelineMessage} so
 * this helper stays trivially unit-testable without constructing full rows.
 */
export type AutoContinueAnchor = Pick<
  TimelineMessage,
  "signerPubkey" | "pubkey" | "author" | "tags"
>;

type ComputeAutoContinueAgentMentionsInput = {
  /**
   * The message the reply attaches to (reply target, else thread head).
   * `null` when there is no resolvable anchor — no auto-continue occurs.
   */
  anchor: AutoContinueAnchor | null | undefined;
  /** Current user's pubkey (the human composing the reply). */
  currentPubkey: string | null | undefined;
  /** Set of known agent pubkeys (normalized lowercase hex). */
  agentPubkeys: ReadonlySet<string> | null | undefined;
  /** Mention pubkeys already resolved for the outgoing reply. */
  existingMentionPubkeys: readonly string[];
};

/**
 * Resolve the raw signer pubkey of an anchor message.
 *
 * Prefers `signerPubkey` (the authenticated event signer) over the display
 * `pubkey`/`author`, which may be overridden by `actor` or `p` tags. Using the
 * signer is essential here: we only auto-continue for messages an agent
 * genuinely authored, never for ones merely displayed under an agent's name.
 */
function anchorSignerPubkey(anchor: AutoContinueAnchor): string | null {
  const raw = anchor.signerPubkey ?? anchor.pubkey ?? anchor.author;
  if (!raw) {
    return null;
  }
  const normalized = normalizePubkey(raw);
  return normalized.length > 0 ? normalized : null;
}

/**
 * Whether the anchor message `p`-tagged the given pubkey.
 */
function anchorMentions(anchor: AutoContinueAnchor, pubkey: string): boolean {
  const tags = anchor.tags ?? [];
  return tags.some(
    (tag) => tag[0] === "p" && normalizePubkey(tag[1] ?? "") === pubkey,
  );
}

/**
 * Decide which agent pubkey(s) to auto-add to a thread reply so the agent
 * loop continues without requiring an explicit @mention.
 *
 * Behaviour (all conditions must hold):
 *  1. The reply is anchored to a message authored by a known agent.
 *  2. That agent message `p`-tagged the current user (i.e. the agent was
 *     addressing / handing the turn back to this human).
 *  3. The reply does not already mention that agent.
 *
 * When satisfied, the agent's pubkey is returned so the caller can merge it
 * into the reply's `mentionPubkeys`. The reply then carries a `["p", agent]`
 * tag, which passes both the relay's mention-gated subscription and the ACP
 * harness `require_mention` filter — starting a fresh agent turn exactly as an
 * explicit @mention would.
 *
 * Returns an empty array when auto-continue does not apply. Pure and
 * side-effect free.
 */
export function computeAutoContinueAgentMentions({
  anchor,
  currentPubkey,
  agentPubkeys,
  existingMentionPubkeys,
}: ComputeAutoContinueAgentMentionsInput): string[] {
  if (!anchor || !currentPubkey || !agentPubkeys || agentPubkeys.size === 0) {
    return [];
  }

  const self = normalizePubkey(currentPubkey);
  if (self.length === 0) {
    return [];
  }

  const agentPubkey = anchorSignerPubkey(anchor);
  if (!agentPubkey || !agentPubkeys.has(agentPubkey)) {
    // The anchor was not authored by a known agent — nothing to continue.
    return [];
  }

  if (agentPubkey === self) {
    // Never auto-mention ourselves (e.g. an agent replying to its own turn).
    return [];
  }

  if (!anchorMentions(anchor, self)) {
    // The agent did not address this user — don't hijack the reply.
    return [];
  }

  const alreadyMentioned = new Set(
    existingMentionPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  if (alreadyMentioned.has(agentPubkey)) {
    return [];
  }

  return [agentPubkey];
}
