/**
 * Agent-reply auto-open policy.
 *
 * When the user just sent a message that targeted an agent and the agent
 * replies, the reply is rendered only inside the thread panel (NIP-10
 * replies are hidden from the main timeline). Without auto-open the user's
 * active exchange visually "leaves" the conversation: a reply summary
 * appears, but the panel stays closed and the user has to click the row to
 * read the answer.
 *
 * The policy is intentionally narrow — it never auto-opens a thread for a
 * human reply, for an agent reply to someone else's thread, or while another
 * auxiliary panel is active. The caller supplies the exact event id of the
 * user's most recent top-level message that targeted an agent; an auto-open
 * only fires when the new agent reply's NIP-10 thread root equals that id.
 * This keeps the policy strict — there is no fuzzy window or author check to
 * silently widen later.
 */
import { getThreadReference, isBroadcastReply } from "./threading.ts";
import type { RelayEvent } from "@/shared/api/types";
import { resolveEventAuthorPubkey } from "@/shared/lib/authors";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";

export type AgentReplyAutoOpenInput = {
  /** Newly received live channel event. */
  event: RelayEvent;
  /** Whether another auxiliary panel (thread, agent session, profile, management) is open. */
  hasActiveAuxiliaryPanel: boolean;
  /** Exact id of the most recent top-level user message that targeted an agent. */
  expectedRootId: string | null;
  /** Pubkeys currently known to represent agents in the active channel. */
  agentPubkeys: ReadonlySet<string>;
  /** NIP-11 relay self pubkey used to validate delegated actor attribution. */
  relaySelfPubkey?: string | null;
};

export type AgentReplyAutoOpenDecision = {
  /** Root id to auto-open. Absent when the policy says no auto-open. */
  rootId?: string;
  /** Reply id that triggered the auto-open. */
  replyId?: string;
};

/**
 * Returns the thread root to auto-open when the live event is a hidden reply
 * from a known agent and its NIP-10 thread root matches `expectedRootId`.
 * Pure function — depends only on its inputs.
 */
export function decideAgentReplyAutoOpen(
  input: AgentReplyAutoOpenInput,
): AgentReplyAutoOpenDecision {
  if (
    input.hasActiveAuxiliaryPanel ||
    input.expectedRootId === null ||
    input.event.kind !== KIND_STREAM_MESSAGE
  ) {
    return {};
  }

  const thread = getThreadReference(input.event.tags ?? []);
  if (thread.parentId === null || isBroadcastReply(input.event.tags ?? [])) {
    return {};
  }
  const rootId = thread.rootId ?? thread.parentId;
  if (rootId !== input.expectedRootId) {
    return {};
  }

  const authorPubkey = normalizePubkey(
    resolveEventAuthorPubkey({
      event: input.event,
      preferActorTag: true,
      relaySelfPubkey: input.relaySelfPubkey,
      requireChannelTagForPTags: true,
    }),
  );
  if (!input.agentPubkeys.has(authorPubkey)) {
    return {};
  }

  return { rootId, replyId: input.event.id };
}
