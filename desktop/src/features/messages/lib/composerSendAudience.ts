/**
 * Merge explicit composer mentions with implicit contextual-agent audience
 * for the outgoing p-tag set.
 */

import {
  resolveContextualAgentConversation,
  type ContextualAgentConversationInput,
  type UnaddressedChannelAgentMode,
} from "@/features/channels/lib/contextualAgentConversationPolicy.ts";

export type ComposerSendAudienceInput = {
  conversation: "direct" | "channel";
  messagePosition: "top-level" | "in-thread";
  unaddressedMode: UnaddressedChannelAgentMode;
  keepAddressedAgentsActive: boolean;
  /** Explicit @mentions (any pubkey) from the draft body. */
  explicitMentionPubkeys: readonly string[];
  /** Explicit mentions that are agents (for policy). */
  explicitAgentPubkeys: readonly string[];
  currentAgentPubkey: string | null;
  channelMemberPubkeys: readonly string[];
  verifiedChannelAgentPubkeys: readonly string[];
  persistentThreadAudience: readonly string[];
  manualRemovedPubkeys?: readonly string[];
  threadRootEventId?: string | null;
  humanMessageEventId?: string | null;
  recipientLoadError?: boolean;
};

export type ComposerSendAudienceResult = {
  /** Full p-tag pubkey list (explicit non-agents + resolved agent audience). */
  mentionPubkeys: string[];
  /** Resolved agent audience only. */
  agentAudiencePubkeys: string[];
  sharedThread: boolean;
  retainDraft: boolean;
  replyPlacement: ReturnType<
    typeof resolveContextualAgentConversation
  >["replyPlacement"];
};

function uniqueNormalized(pubkeys: Iterable<string>): string[] {
  return [
    ...new Set(
      [...pubkeys].map((pk) => pk.trim().toLowerCase()).filter(Boolean),
    ),
  ];
}

/**
 * Build the effective send audience for a human message.
 * Non-agent explicit mentions are always preserved; agent audience follows policy.
 */
export function resolveComposerSendAudience(
  input: ComposerSendAudienceInput,
): ComposerSendAudienceResult {
  const explicitAgentSet = new Set(
    uniqueNormalized(input.explicitAgentPubkeys),
  );
  const policyInput: ContextualAgentConversationInput = {
    conversation: input.conversation,
    messagePosition: input.messagePosition,
    senderClass: "human",
    unaddressedMode: input.unaddressedMode,
    keepAddressedAgentsActive: input.keepAddressedAgentsActive,
    explicitMentionPubkeys: [...explicitAgentSet],
    currentAgentPubkey: input.currentAgentPubkey,
    channelMemberPubkeys: [...input.channelMemberPubkeys],
    verifiedChannelAgentPubkeys: [...input.verifiedChannelAgentPubkeys],
    threadRootEventId: input.threadRootEventId ?? null,
    persistentThreadAudience: [...input.persistentThreadAudience],
    manualRemovedPubkeys: [...(input.manualRemovedPubkeys ?? [])],
    recipientLoadError: input.recipientLoadError ?? false,
    humanMessageEventId: input.humanMessageEventId ?? null,
  };

  const decision = resolveContextualAgentConversation(policyInput);
  // Always retain authored agent @mentions (e.g. DM expansion to a new agent)
  // while still applying implicit/persistent audience from policy.
  const agentAudience = uniqueNormalized([
    ...decision.audiencePubkeys,
    ...explicitAgentSet,
  ]);
  const humanMentions = uniqueNormalized(input.explicitMentionPubkeys).filter(
    (pk) => !explicitAgentSet.has(pk),
  );
  const mentionPubkeys = uniqueNormalized([...humanMentions, ...agentAudience]);

  return {
    mentionPubkeys,
    agentAudiencePubkeys: agentAudience,
    sharedThread: decision.sharedThread || agentAudience.length >= 2,
    retainDraft: decision.retainDraft,
    replyPlacement: decision.replyPlacement,
  };
}

/** Human-readable composer hint for the unaddressed mode + draft state. */
export function describeComposerAudienceHint({
  conversation,
  unaddressedMode,
  explicitAgentCount,
  implicitAgentCount,
  retainDraft,
}: {
  conversation: "direct" | "channel";
  unaddressedMode: UnaddressedChannelAgentMode;
  explicitAgentCount: number;
  implicitAgentCount: number;
  retainDraft: boolean;
}): string | null {
  if (retainDraft) {
    return "Could not resolve recipients — draft kept";
  }
  if (conversation === "direct") {
    return null;
  }
  if (explicitAgentCount > 0) {
    return explicitAgentCount === 1
      ? "Notifying 1 mentioned agent"
      : `Notifying ${explicitAgentCount} mentioned agents`;
  }
  if (implicitAgentCount > 0 && unaddressedMode === "all-channel-agents") {
    return implicitAgentCount === 1
      ? "Notifying 1 channel agent"
      : `Notifying all ${implicitAgentCount} channel agents`;
  }
  if (unaddressedMode === "mentions-only") {
    return "Mentions only — agents are not auto-notified";
  }
  return null;
}
