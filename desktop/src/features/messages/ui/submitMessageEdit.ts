import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import { enqueueBackgroundMediaUpload } from "@/features/messages/lib/backgroundMediaUploadStore";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import {
  buildOutgoingMessage,
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { diffAddedMentionPubkeys } from "@/features/messages/lib/threading";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

type EditDraft = {
  content: string;
  mentionRefs: DraftMentionRef[];
  pendingImeta: ImetaMedia[];
  queuedAttachments: QueuedMediaAttachment[];
  spoileredAttachmentUrls: Set<string>;
};

type SubmitMessageEditOptions = Omit<EditDraft, "mentionRefs"> & {
  clearComposer: () => void;
  customEmoji: ReadonlyArray<CustomEmoji>;
  extractMentionPubkeys: (content: string) => string[];
  getMentionRefs: (content: string) => DraftMentionRef[];
  editTargetId: string;
  originalContent: string;
  ownerPubkey: string | null;
  restoreComposer: (draft: EditDraft) => void;
  restoreMentionRefs: (refs: DraftMentionRef[]) => void;
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

/** Clear an edited message immediately, then upload and save captured state. */
export async function submitMessageEdit({
  clearComposer,
  content,
  customEmoji,
  editTargetId,
  extractMentionPubkeys,
  getMentionRefs,
  originalContent,
  ownerPubkey,
  pendingImeta,
  queuedAttachments,
  restoreComposer,
  restoreMentionRefs,
  setDeferredUploadPending,
  shouldRestoreComposer,
  save,
  setUploadError,
  spoileredAttachmentUrls,
}: SubmitMessageEditOptions): Promise<void> {
  const draft: EditDraft = {
    content,
    mentionRefs: getMentionRefs(content),
    pendingImeta: [...pendingImeta],
    queuedAttachments: [...queuedAttachments],
    spoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
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
    const outgoingTags =
      mergeOutgoingTags(
        mediaTags,
        buildCustomEmojiTags(finalContent, customEmoji),
      ) ?? [];
    if (signal?.aborted) return;
    await save(finalContent, outgoingTags, addedMentionPubkeys, editTargetId);
  };

  if (hasQueuedAttachments) {
    enqueueBackgroundMediaUpload({
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
