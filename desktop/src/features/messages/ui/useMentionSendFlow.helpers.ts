import type { ManagedAgent } from "@/shared/api/types";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { MENTION_REFERENCE_TAG } from "@/shared/lib/resolveMentionNames";

export { MENTION_REFERENCE_TAG };

export type PendingNonMemberMentionSend = {
  capturedChannelId: string | null;
  capturedThreadContext: {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
  trimmed: string;
  mentionPubkeys: string[];
  nonMemberPubkeys: string[];
  outgoingTags?: string[][];
  preparedManagedAgents?: ManagedAgent[];
  readyAgentPubkeys?: string[];
  savedContent: string;
  savedImeta: ImetaMedia[];
  queuedAttachments: QueuedMediaAttachment[];
  savedSpoileredAttachmentUrls: Set<string>;
  sentDraftKey: string | null | undefined;
  recoveryDraftKey: string | null | undefined;
  savedMentionRefs: DraftMentionRef[];
  audienceGeneration: number;
  audienceRevision: number | null;
  explicitAgentPubkeys: string[];
};

export type SendMessageWithMentionFlowInput = {
  capturedChannelId: string | null;
  capturedThreadContext?: PendingNonMemberMentionSend["capturedThreadContext"];
  pendingImeta: ImetaMedia[];
  queuedAttachments?: QueuedMediaAttachment[];
  linkPreviewTags?: string[][];
  sentDraftKey: string | null | undefined;
  recoveryDraftKey: string | null | undefined;
  spoileredAttachmentUrls?: ReadonlySet<string>;
  trimmed: string;
  audienceGeneration?: number;
  audienceRevision?: number | null;
};

export function mergeOutgoingTagsWithReferenceMentions(
  outgoingTags: string[][] | undefined,
  pubkeys: Iterable<string>,
) {
  const normalizedPubkeys = uniqueNormalizedPubkeys(pubkeys);
  if (normalizedPubkeys.length === 0) {
    return outgoingTags;
  }

  return [
    ...(outgoingTags ?? []),
    ...normalizedPubkeys.map((pubkey) => [MENTION_REFERENCE_TAG, pubkey]),
  ];
}

export function buildMentionTags(
  content: string,
  customEmoji: CustomEmoji[],
  getTeamMentionTags: (content: string) => string[][],
) {
  return [
    ...buildCustomEmojiTags(content, customEmoji),
    ...getTeamMentionTags(content),
  ];
}

export function getErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

export function uniqueNormalizedPubkeys(pubkeys: Iterable<string>) {
  return [...new Set([...pubkeys].map(normalizePubkey))].filter(Boolean);
}

export const MAX_MENTION_PUBKEYS = 50;

export function exceedsMentionPubkeyLimit(
  resolvedPubkeys: Iterable<string>,
  unresolvedPersonaIds: Iterable<string>,
) {
  const targetPubkeys = new Set(
    [...resolvedPubkeys].map(normalizePubkey).filter(Boolean),
  );
  for (const personaId of unresolvedPersonaIds) {
    if (personaId.trim()) targetPubkeys.add(`persona:${personaId}`);
  }
  return targetPubkeys.size > MAX_MENTION_PUBKEYS;
}

export function isManagedAgentRunning(agent: ManagedAgent) {
  return agent.status === "running" || agent.status === "deployed";
}

export function isProviderBackedAgent(agent: ManagedAgent) {
  return agent.backend.type === "provider";
}
