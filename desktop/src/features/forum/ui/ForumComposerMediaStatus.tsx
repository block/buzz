import * as React from "react";

import type { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { ComposerAttachments } from "@/features/messages/ui/ComposerAttachments";

type ComposerMedia = Pick<
  ReturnType<typeof useMediaUpload>,
  | "isUploading"
  | "cancelUpload"
  | "originalUrlByUrl"
  | "pendingImeta"
  | "removeAttachment"
  | "revertAttachment"
  | "setUploadState"
  | "uploadEditedAttachment"
  | "uploadState"
  | "uploadingCount"
  | "uploadingPreviews"
>;

type ForumComposerMediaStatusProps = {
  media: ComposerMedia;
};

export function ForumComposerMediaStatus({
  media,
}: ForumComposerMediaStatusProps) {
  const handleEditSave = React.useCallback(
    async (url: string, bytes: Uint8Array) => {
      await media.uploadEditedAttachment(url, bytes);
    },
    [media.uploadEditedAttachment],
  );

  return (
    <>
      {media.uploadState.status === "error" ? (
        <div className="mb-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
          Upload failed: {media.uploadState.message}
          <button
            className="ml-2 underline"
            onClick={() => media.setUploadState({ status: "idle" })}
            type="button"
          >
            Dismiss
          </button>
        </div>
      ) : null}

      {(media.pendingImeta.length > 0 || media.isUploading) && (
        <div className="mb-2 flex items-center gap-2">
          <ComposerAttachments
            attachments={media.pendingImeta}
            isUploading={media.isUploading}
            onCancelUpload={media.cancelUpload}
            onEditSave={handleEditSave}
            onRemove={media.removeAttachment}
            onRevert={media.revertAttachment}
            originalUrlByUrl={media.originalUrlByUrl}
            uploadingCount={media.uploadingCount}
            uploadingPreviews={media.uploadingPreviews}
          />
        </div>
      )}
    </>
  );
}
