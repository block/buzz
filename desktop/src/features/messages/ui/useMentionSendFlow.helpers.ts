import type { ManagedAgent } from "@/shared/api/types";
import {
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import type { PreparedBackgroundLinkPreviews } from "@/features/messages/lib/linkPreviewPreparationStore";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MENTION_REFERENCE_TAG } from "@/shared/lib/resolveMentionNames";

export { MENTION_REFERENCE_TAG };

export type PendingNonMemberMentionSend = {
  addressedAgentPubkeys: string[];
  inlineAgentMentionPubkeys: string[];
  capturedChannelId: string | null;
  capturedThreadContext: {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
  trimmed: string;
  mentionPubkeys: string[];
  nonMemberPubkeys: string[];
  outgoingTags?: string[][];
  preparedLinkPreviews?: PreparedBackgroundLinkPreviews | null;
  preparedManagedAgents?: ManagedAgent[];
  readyAgentPubkeys?: string[];
  savedContent: string;
  savedImeta: ImetaMedia[];
  queuedAttachments: QueuedMediaAttachment[];
  savedSpoileredAttachmentUrls: Set<string>;
  sentDraftKey: string | null | undefined;
  recoveryDraftKey: string | null | undefined;
  savedMentionRefs: DraftMentionRef[];
};

export type SendMessageWithMentionFlowInput = {
  addressedAgentPubkeys?: readonly string[];
  capturedChannelId: string | null;
  capturedThreadContext?: PendingNonMemberMentionSend["capturedThreadContext"];
  pendingImeta: ImetaMedia[];
  queuedAttachments?: QueuedMediaAttachment[];
  linkPreviewTags?: string[][];
  preparedLinkPreviews?: PreparedBackgroundLinkPreviews | null;
  sentDraftKey: string | null | undefined;
  recoveryDraftKey: string | null | undefined;
  spoileredAttachmentUrls?: ReadonlySet<string>;
  trimmed: string;
};

export async function resolvePreviewTags(
  draft: Pick<PendingNonMemberMentionSend, "preparedLinkPreviews">,
  mediaTags: string[][] | undefined,
  outgoingTags: string[][] | undefined,
): Promise<string[][] | null> {
  const result = await draft.preparedLinkPreviews?.promise;
  if (result?.status === "cancelled") return null;
  return (
    mergeOutgoingTags(mediaTags, [
      ...(outgoingTags ?? []),
      ...(result?.tags ?? []),
    ]) ?? []
  );
}

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

export function getErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

export function uniqueNormalizedPubkeys(pubkeys: Iterable<string>) {
  return [...new Set([...pubkeys].map(normalizePubkey))].filter(Boolean);
}

export function mergeMentionRecipients(
  explicitMentionPubkeys: Iterable<string>,
  addressedAgentPubkeys: Iterable<string>,
) {
  return uniqueNormalizedPubkeys([
    ...explicitMentionPubkeys,
    ...addressedAgentPubkeys,
  ]);
}

export function isManagedAgentRunning(agent: ManagedAgent) {
  return agent.status === "running" || agent.status === "deployed";
}

export function isProviderBackedAgent(agent: ManagedAgent) {
  return agent.backend.type === "provider";
}
