/**
 * Sticker-send helpers for `MessageComposer`.
 *
 * Split out of `MessageComposer.tsx` to keep that module under the desktop
 * file-size ceiling (see `desktop/scripts/check-file-sizes.mjs`), and to give
 * the sticker send path a unit-testable seam.
 *
 * A sticker send is *not* a normal send with an extra tag: renderers
 * (`MessageRow.renderBody` on desktop, `MessageContent` on mobile) take the
 * sticker branch and drop the markdown body entirely. So any typed text or
 * pending attachment that rode along would be invisible to recipients. These
 * helpers therefore send the sticker alone and hand the untouched draft back to
 * the composer afterwards.
 */
import type { StickerSelection } from "@/features/stickers/ui/ComposerStickerPicker";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { stickerReferenceTag } from "@/shared/api/stickers";

/** Draft state captured before a sticker send clears the composer. */
export type StickerDraftSnapshot = {
  content: string;
  imeta: ImetaMedia[];
  spoileredAttachmentUrls: Set<string>;
};

/** The subset of send-flow arguments a sticker overrides. */
export type StickerSendOverrides = {
  pendingImeta: ImetaMedia[];
  spoileredAttachmentUrls: Set<string>;
  trimmed: string;
  stickerTags: string[][];
};

/**
 * Build the send-flow arguments for this submit.
 *
 * With no sticker this is the identity case (the caller's own draft state).
 * With a sticker the message becomes the sticker alone: the shortcode is the
 * text fallback for clients that cannot resolve the pack, and attachments are
 * withheld rather than silently hidden behind the sticker branch.
 */
export function stickerSendOverrides(
  immediateSticker: StickerSelection | undefined,
  currentPendingImeta: ImetaMedia[],
  spoileredAttachmentUrls: Set<string>,
  trimmed: string,
): StickerSendOverrides {
  if (!immediateSticker) {
    return {
      pendingImeta: currentPendingImeta,
      spoileredAttachmentUrls,
      trimmed,
      stickerTags: [],
    };
  }
  return {
    pendingImeta: [],
    spoileredAttachmentUrls: new Set<string>(),
    trimmed: `:${immediateSticker.sticker.shortcode}:`,
    stickerTags: [
      stickerReferenceTag(immediateSticker.pack, immediateSticker.sticker),
    ],
  };
}

/**
 * Capture the draft a sticker send is about to displace, or `null` when this
 * submit is not a sticker send (nothing to restore afterwards).
 */
export function captureStickerDraft(
  immediateSticker: StickerSelection | undefined,
  content: string,
  currentPendingImeta: ImetaMedia[],
  spoileredAttachmentUrls: Set<string>,
): StickerDraftSnapshot | null {
  if (!immediateSticker) return null;
  return {
    content,
    imeta: [...currentPendingImeta],
    spoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
  };
}

/**
 * Whether the pre-send draft should be restored into the composer.
 *
 * The send flow clears the composer on success, so a sticker click would
 * otherwise eat an unsent draft. But the editor stays interactive while the
 * request is in flight (hosts such as `InboxDetailPane` keep it enabled and
 * track sending separately), so an unconditional restore would clobber
 * whatever the user typed meanwhile. Restore only when the composer is still
 * in the state the send left it — empty.
 */
export function shouldRestoreStickerDraft(
  snapshot: StickerDraftSnapshot | null,
  liveContent: string,
): snapshot is StickerDraftSnapshot {
  return snapshot !== null && liveContent.trim() === "";
}
