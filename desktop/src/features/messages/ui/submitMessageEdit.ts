import { snapshotUnresolvedEditMentionPubkeys } from "@/features/messages/lib/draftMentionRefs";
import {
  AgentMentionAuthorizationError,
  type MentionRevalidationOptions,
} from "@/features/messages/lib/agentMentionRevalidation";
import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import { enqueueBackgroundMediaUpload } from "@/features/messages/lib/backgroundMediaUploadStore";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import type { MessageComposerEditTarget } from "@/features/messages/ui/MessageComposer.types";
import {
  buildOutgoingMessage,
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { diffAddedMentionPubkeys } from "@/features/messages/lib/threading";
import { mergeOutgoingTagsWithReferenceMentions } from "@/features/messages/ui/useMentionSendFlow.helpers";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

type EditDraft = {
  content: string;
  mentionRefs: DraftMentionRef[];
  pendingImeta: ImetaMedia[];
  queuedAttachments: QueuedMediaAttachment[];
  spoileredAttachmentUrls: Set<string>;
  unresolvedMentionPubkeys: string[];
};

type SubmitMessageEditOptions = Omit<
  EditDraft,
  "mentionRefs" | "unresolvedMentionPubkeys"
> & {
  clearComposer: () => void;
  customEmoji: ReadonlyArray<CustomEmoji>;
  extractMentionPubkeys: (
    content: string,
    competingDisplayNames?: readonly string[],
  ) => string[];
  getMentionRefs: (
    content: string,
    fallbackRefs: readonly DraftMentionRef[],
    competingDisplayNames?: readonly string[],
  ) => DraftMentionRef[];
  editTargetId: string;
  enqueueUpload?: typeof enqueueBackgroundMediaUpload;
  editTarget: Pick<
    MessageComposerEditTarget,
    "mentionRefs" | "unresolvedMentionPubkeys" | "unresolvedMentionRefs"
  >;
  originalContent: string;
  ownerPubkey: string | null;
  restoreComposer: (draft: EditDraft) => void;
  restoreMentionRefs: (refs: DraftMentionRef[]) => void;
  revalidateMentionPubkeys: (
    pubkeys: readonly string[],
    channelId?: string | null,
    options?: MentionRevalidationOptions,
  ) => Promise<string[]>;
  shouldRestoreComposer: () => boolean;
  setDeferredUploadPending: (isPending: boolean) => void;
  save: (
    content: string,
    mediaTags?: string[][],
    mentionPubkeys?: string[],
    eventId?: string,
  ) => Promise<void>;
  setUploadError: (message: string) => void;
};

export async function submitMessageEdit({
  clearComposer,
  content,
  customEmoji,
  editTargetId,
  enqueueUpload = enqueueBackgroundMediaUpload,
  editTarget,
  extractMentionPubkeys,
  getMentionRefs,
  originalContent,
  ownerPubkey,
  pendingImeta,
  queuedAttachments,
  restoreComposer,
  restoreMentionRefs,
  revalidateMentionPubkeys,
  setDeferredUploadPending,
  shouldRestoreComposer,
  save,
  setUploadError,
  spoileredAttachmentUrls,
}: SubmitMessageEditOptions): Promise<void> {
  const historicalNames = (editTarget.unresolvedMentionRefs ?? []).map(
    (ref) => ref.displayName,
  );
  const draft: EditDraft = {
    content,
    mentionRefs: getMentionRefs(
      content,
      editTarget.mentionRefs ?? [],
      historicalNames,
    ),
    pendingImeta: [...pendingImeta],
    queuedAttachments: [...queuedAttachments],
    spoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
    unresolvedMentionPubkeys: snapshotUnresolvedEditMentionPubkeys(
      content,
      originalContent,
      editTarget,
      getMentionRefs,
    ),
  };
  const restoreDraft = () => {
    if (shouldRestoreComposer()) {
      restoreComposer(draft);
      restoreMentionRefs(draft.mentionRefs);
    }
  };
  // Current picker bindings must not reinterpret the original body: selecting a
  // different Scout would otherwise make that new key look already notified.
  // With unresolved history, conservatively revalidate all current recipients.
  const originalMentionPubkeys = editTarget.unresolvedMentionPubkeys?.length
    ? []
    : (editTarget.mentionRefs ?? []).map((ref) => ref.pubkey);
  let addedMentionPubkeys: string[];
  try {
    addedMentionPubkeys = diffAddedMentionPubkeys(
      originalMentionPubkeys,
      extractMentionPubkeys(content, historicalNames),
      ownerPubkey ?? "",
    );
  } catch (error) {
    setUploadError(error instanceof Error ? error.message : String(error));
    return;
  }
  const hasQueuedAttachments = draft.queuedAttachments.length > 0;
  if (hasQueuedAttachments) setDeferredUploadPending(true);
  clearComposer();

  const finishEdit = async (uploaded: ImetaMedia[], signal?: AbortSignal) => {
    // An explicit empty media tag set tells edit receivers to wipe attachments.
    const { content: finalContent, mediaTags } = buildOutgoingMessage(
      content,
      [...draft.pendingImeta, ...uploaded],
      new Set([
        ...draft.spoileredAttachmentUrls,
        ...draft.queuedAttachments.flatMap((attachment, index) =>
          attachment.spoilered && uploaded[index] ? [uploaded[index].url] : [],
        ),
      ]),
    );
    if (signal?.aborted) return;
    const revalidatedMentionPubkeys = await revalidateMentionPubkeys(
      addedMentionPubkeys,
      undefined,
      {
        intendedAgentPubkeys: draft.mentionRefs
          .filter((ref) => ref.isAgent)
          .map((ref) => ref.pubkey),
      },
    );
    if (signal?.aborted) return;
    const outgoingTags = mergeOutgoingTagsWithReferenceMentions(
      mergeOutgoingTags(
        mediaTags,
        buildCustomEmojiTags(finalContent, customEmoji),
      ),
      [
        ...draft.mentionRefs.map(({ pubkey }) => pubkey),
        ...draft.unresolvedMentionPubkeys,
        // Newly typed recipients are not necessarily selected draft refs. The
        // authoritative snapshot must include them too, but only after relay
        // eligibility revalidation, so forwarding cannot resurrect old p-tags.
        ...revalidatedMentionPubkeys,
      ],
    );
    await save(
      finalContent,
      outgoingTags,
      revalidatedMentionPubkeys,
      editTargetId,
    );
  };

  if (hasQueuedAttachments) {
    enqueueUpload({
      attachments: draft.queuedAttachments,
      onComplete: async (uploaded, signal) => {
        try {
          await finishEdit(uploaded, signal);
        } catch (error) {
          restoreDraft();
          if (error instanceof AgentMentionAuthorizationError)
            setUploadError(error.message);
        } finally {
          setDeferredUploadPending(false);
        }
      },
      onError: (error) => {
        restoreDraft();
        setUploadError(String(error));
        setDeferredUploadPending(false);
      },
      onCancel: () => {
        restoreDraft();
        setDeferredUploadPending(false);
      },
    });
    return;
  }

  try {
    await finishEdit([]);
  } catch (error) {
    restoreDraft();
    if (error instanceof AgentMentionAuthorizationError)
      setUploadError(error.message);
  }
}
