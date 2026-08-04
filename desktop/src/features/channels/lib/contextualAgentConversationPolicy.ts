/**
 * Contextual agent audience + reply-placement policy (Desktop).
 *
 * Contract: tests/fixtures/contextual-agent-conversation-cases.json
 *
 * Pure resolver — no I/O. Device-local mode persistence lives in
 * `unaddressedChannelAgentMode.ts`.
 */

import { normalizePubkey } from "@/shared/lib/pubkey.ts";

export type UnaddressedChannelAgentMode =
  | "all-channel-agents"
  | "mentions-only";

export type ReplyPlacement =
  | { kind: "top-level" }
  | { kind: "thread-root"; eventId: string }
  | { kind: "unconstrained" };

export type ContextualAgentConversationInput = {
  conversation: "direct" | "channel";
  messagePosition: "top-level" | "in-thread";
  senderClass: "human" | "agent";
  unaddressedMode: UnaddressedChannelAgentMode;
  keepAddressedAgentsActive: boolean;
  explicitMentionPubkeys: string[];
  currentAgentPubkey: string | null;
  channelMemberPubkeys: string[];
  verifiedChannelAgentPubkeys: string[];
  unverifiedAgentPubkeys?: string[];
  nonMemberAgentPubkeys?: string[];
  threadRootEventId: string | null;
  replyingUnderEventId?: string | null;
  persistentThreadAudience: string[];
  manualRemovedPubkeys: string[];
  recipientLoadError: boolean;
  /** Id of the human message that becomes a multi-agent thread root. */
  humanMessageEventId?: string | null;
};

export type ContextualAgentConversationDecision = {
  audiencePubkeys: string[];
  replyPlacement: ReplyPlacement;
  sharedThread: boolean;
  retainDraft: boolean;
  nestUnderAgentReply?: boolean;
};

function uniqueSorted(pubkeys: Iterable<string>): string[] {
  const set = new Set<string>();
  for (const pk of pubkeys) {
    const n = normalizePubkey(pk);
    if (n) set.add(n);
  }
  return [...set].sort();
}

function asSet(pubkeys: readonly string[]): Set<string> {
  return new Set(pubkeys.map(normalizePubkey).filter(Boolean));
}

/**
 * Verified current-channel agents only: intersection of membership and
 * verified agent evidence. Never community/relay-wide fanout.
 */
function eligibleChannelAgents(
  input: ContextualAgentConversationInput,
): Set<string> {
  const members = asSet(input.channelMemberPubkeys);
  const verified = asSet(input.verifiedChannelAgentPubkeys);
  const eligible = new Set<string>();
  for (const pk of verified) {
    if (members.has(pk)) eligible.add(pk);
  }
  return eligible;
}

function filterToEligible(
  candidates: readonly string[],
  eligible: Set<string>,
): string[] {
  return uniqueSorted(
    candidates.filter((pk) => eligible.has(normalizePubkey(pk))),
  );
}

/**
 * Resolve audience and reply placement for a human/agent send path.
 *
 * Precedence:
 * 1. Recipient-load errors fail closed (retain draft).
 * 2. Agent-authored traffic is unconstrained.
 * 3. Explicit @mentions (eligible agents only).
 * 4. Persistent audience when Keep addressed agents active (minus manual removals).
 * 5. Unaddressed mode: all verified channel agents, or mentions-only (none).
 * 6. Direct conversations always resolve to the current agent.
 */
export function resolveContextualAgentConversation(
  input: ContextualAgentConversationInput,
): ContextualAgentConversationDecision {
  if (input.recipientLoadError) {
    return {
      audiencePubkeys: [],
      replyPlacement: { kind: "top-level" },
      sharedThread: false,
      retainDraft: true,
      nestUnderAgentReply: false,
    };
  }

  if (input.senderClass === "agent") {
    return {
      audiencePubkeys: [],
      replyPlacement: { kind: "unconstrained" },
      sharedThread: false,
      retainDraft: false,
      nestUnderAgentReply: false,
    };
  }

  // Human path
  if (input.conversation === "direct") {
    const current = input.currentAgentPubkey
      ? normalizePubkey(input.currentAgentPubkey)
      : null;
    const audience = current ? [current] : [];
    return {
      audiencePubkeys: audience,
      replyPlacement: placementFor(input, audience.length),
      sharedThread: false,
      retainDraft: false,
      nestUnderAgentReply: false,
    };
  }

  // Channel path
  const eligible = eligibleChannelAgents(input);
  const removed = asSet(input.manualRemovedPubkeys);
  const explicit = filterToEligible(
    input.explicitMentionPubkeys,
    eligible,
  ).filter((pk) => !removed.has(pk));

  let audience: string[];

  if (explicit.length > 0) {
    audience = explicit;
  } else {
    const persistent = input.keepAddressedAgentsActive
      ? filterToEligible(input.persistentThreadAudience, eligible).filter(
          (pk) => !removed.has(pk),
        )
      : [];

    if (persistent.length > 0) {
      audience = persistent;
    } else if (input.unaddressedMode === "all-channel-agents") {
      audience = uniqueSorted(eligible).filter((pk) => !removed.has(pk));
    } else {
      // mentions-only, no explicit, no persistent
      audience = [];
    }
  }

  const sharedThread = audience.length >= 2;
  return {
    audiencePubkeys: audience,
    replyPlacement: placementFor(input, audience.length),
    sharedThread,
    retainDraft: false,
    nestUnderAgentReply: false,
  };
}

function placementFor(
  input: ContextualAgentConversationInput,
  audienceCount: number,
): ReplyPlacement {
  // Already in a thread: always continue at the existing root (never nest under
  // an agent reply).
  if (input.messagePosition === "in-thread" && input.threadRootEventId) {
    return {
      kind: "thread-root",
      eventId: input.threadRootEventId,
    };
  }

  // Multi-agent top-level human message creates one shared thread at the human
  // event.
  if (audienceCount >= 2) {
    const eventId = input.humanMessageEventId ?? input.threadRootEventId;
    if (eventId) {
      return { kind: "thread-root", eventId };
    }
  }

  return { kind: "top-level" };
}
