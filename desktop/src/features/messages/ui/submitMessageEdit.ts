import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import { enqueueBackgroundMediaUpload } from "@/features/messages/lib/backgroundMediaUploadStore";
import { hasMention } from "@/features/messages/lib/hasMention";
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
  extractMentionPubkeys: (content: string) => string[];
  getMentionRefs: (content: string) => DraftMentionRef[];
  editTargetId: string;
  enqueueUpload?: typeof enqueueBackgroundMediaUpload;
  editTarget: Pick<
    MessageComposerEditTarget,
    "mentionRefs" | "unresolvedMentionPubkeys"
  >;
  originalContent: string;
  ownerPubkey: string | null;
  restoreComposer: (draft: EditDraft) => void;
  restoreMentionRefs: (refs: DraftMentionRef[]) => void;
  revalidateMentionPubkeys: (pubkeys: readonly string[]) => Promise<string[]>;
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
  const currentMentionRefs = editTarget.mentionRefs ?? [];
  const draft: EditDraft = {
    content,
    mentionRefs: [
      ...getMentionRefs(content),
      ...currentMentionRefs.filter((ref) =>
        hasMention(content, ref.displayName),
      ),
    ],
    pendingImeta: [...pendingImeta],
    queuedAttachments: [...queuedAttachments],
    spoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
    unresolvedMentionPubkeys: [...(editTarget.unresolvedMentionPubkeys ?? [])],
  };
  const restoreDraft = () => {
    if (shouldRestoreComposer()) {
      restoreComposer(draft);
      restoreMentionRefs(draft.mentionRefs);
    }
  };
  const addedMentionPubkeys = diffAddedMentionPubkeys(
    extractMentionPubkeys(originalContent),
    extractMentionPubkeys(content),
    ownerPubkey ?? "",
  );
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
    const outgoingTags = mergeOutgoingTagsWithReferenceMentions(
      mergeOutgoingTags(
        mediaTags,
        buildCustomEmojiTags(finalContent, customEmoji),
      ),
      [
        ...draft.mentionRefs.map(({ pubkey }) => pubkey),
        ...draft.unresolvedMentionPubkeys,
      ],
    );
    if (signal?.aborted) return;
    const revalidatedMentionPubkeys =
      await revalidateMentionPubkeys(addedMentionPubkeys);
    if (signal?.aborted) return;
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
        } catch {
          restoreDraft();
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
  } catch {
    restoreDraft();
  }
}
