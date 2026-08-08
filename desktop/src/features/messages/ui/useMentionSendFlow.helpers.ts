import type { ManagedAgent } from "@/shared/api/types";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import { normalizePubkey } from "@/shared/lib/pubkey";
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

export function getErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

export function uniqueNormalizedPubkeys(pubkeys: Iterable<string>) {
  return [...new Set([...pubkeys].map(normalizePubkey))].filter(Boolean);
}

export function isManagedAgentRunning(agent: ManagedAgent) {
  return agent.status === "running" || agent.status === "deployed";
}

export function isProviderBackedAgent(agent: ManagedAgent) {
  return agent.backend.type === "provider";
}

/**
 * A mentioned agent that could not be prepared for the send.
 *
 * `blocking` is false only when the agent is already a member of the channel
 * the message is going to. Launching it on this desktop is then an
 * optimization: an agent that runs elsewhere — a container on the user's own
 * server — has no private key on this machine and never will, so its launch
 * fails every time, yet it is in the channel and still answers. Refusing to
 * publish loses a valid mention and fixes nothing.
 *
 * Everything else blocks, because nothing else would put the agent in the
 * channel: a failed attach leaves it outside, and a failed launch for an agent
 * that is only being prepared for a channel this send creates or expands
 * leaves that channel without the participant it was expanded for.
 */
export type AgentReadinessFailure = {
  blocking: boolean;
  message: string;
};

/**
 * Build the send-blocking message and the send-anyway notice, or null.
 *
 * The non-blocking copy is deliberately pre-send tense. It is shown before
 * Huddle sync, media upload and the send itself, any of which can still fail
 * and abort, so it must not claim the message was delivered.
 */
export function describeAgentReadinessFailures(
  failures: readonly AgentReadinessFailure[],
) {
  const describe = (
    singular: string,
    plural: string,
    selected: readonly AgentReadinessFailure[],
  ) =>
    selected.length === 0
      ? null
      : `${selected.length === 1 ? singular : plural}: ${selected
          .map((failure) => failure.message)
          .join("; ")}`;

  return {
    blocking: describe(
      "Could not start agent mention",
      "Could not start agent mentions",
      failures.filter((failure) => failure.blocking),
    ),
    warning: describe(
      "Could not start the mentioned agent; sending the mention anyway",
      "Could not start the mentioned agents; sending the mention anyway",
      failures.filter((failure) => !failure.blocking),
    ),
  };
}
