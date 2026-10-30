import * as React from "react";
import { useAttachmentEditing } from "@/features/messages/lib/useAttachmentEditing";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";

type UseComposerAttachmentSpoilersArgs = {
  removeAttachment: MediaUploadController["removeAttachment"];
  revertAttachment: MediaUploadController["revertAttachment"];
  uploadEditedAttachment: MediaUploadController["uploadEditedAttachment"];
};

/**
 * Owns which composer attachments are marked as spoilers, and keeps that set
 * consistent with the attachment lifecycle: removing an attachment drops its
 * membership, and editing or reverting one carries membership over to the
 * replacement URL. The mirrored ref lets draft persistence read the current
 * set without re-subscribing to state.
 */
export function useComposerAttachmentSpoilers({
  removeAttachment,
  revertAttachment,
  uploadEditedAttachment,
}: UseComposerAttachmentSpoilersArgs) {
  const [spoileredAttachmentUrls, setSpoileredAttachmentUrls] = React.useState<
    Set<string>
  >(() => new Set());
  const spoileredAttachmentUrlsRef = React.useRef(spoileredAttachmentUrls);
  spoileredAttachmentUrlsRef.current = spoileredAttachmentUrls;

  const handleRemoveAttachment = React.useCallback(
    (url: string) => {
      setSpoileredAttachmentUrls((current) => {
        if (!current.has(url)) return current;
        const next = new Set(current);
        next.delete(url);
        return next;
      });
      removeAttachment(url);
    },
    [removeAttachment],
  );

  const handleToggleAttachmentSpoiler = React.useCallback((url: string) => {
    setSpoileredAttachmentUrls((current) => {
      const next = new Set(current);
      if (next.has(url)) {
        next.delete(url);
      } else {
        next.add(url);
      }
      return next;
    });
  }, []);

  const { handleAttachmentEditSave, handleAttachmentRevert } =
    useAttachmentEditing({
      revertAttachment,
      setSpoileredAttachmentUrls,
      uploadEditedAttachment,
    });

  return {
    handleAttachmentEditSave,
    handleAttachmentRevert,
    handleRemoveAttachment,
    handleToggleAttachmentSpoiler,
    setSpoileredAttachmentUrls,
    spoileredAttachmentUrls,
    spoileredAttachmentUrlsRef,
  };
}
