import * as React from "react";

import type { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { ComposerAttachments } from "@/features/messages/ui/ComposerAttachments";
import { useTranslation } from "@/i18n";

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
  disabled?: boolean;
  media: ComposerMedia;
};

export function ForumComposerMediaStatus({
  disabled = false,
  media,
}: ForumComposerMediaStatusProps) {
  const { t } = useTranslation();
  const handleEditSave = React.useCallback(
    async (url: string, bytes: Uint8Array) => {
      if (disabled) return;
      await media.uploadEditedAttachment(url, bytes);
    },
    [disabled, media.uploadEditedAttachment],
  );
  const guard = React.useCallback(
    <T,>(callback: (value: T) => void) =>
      (value: T) => {
        if (!disabled) callback(value);
      },
    [disabled],
  );

  return (
    <>
      {media.uploadState.status === "error" ? (
        <div className="mb-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {t("messages.composer.upload-failed", {
            message: media.uploadState.message,
          })}
          <button
            className="ml-2 underline"
            disabled={disabled}
            onClick={() => media.setUploadState({ status: "idle" })}
            type="button"
          >
            {t("messages.composer.dismiss")}
          </button>
        </div>
      ) : null}

      {(media.pendingImeta.length > 0 || media.isUploading) && (
        <div className="mb-2 flex items-center gap-2">
          <ComposerAttachments
            attachments={media.pendingImeta}
            isUploading={media.isUploading}
            onCancelUpload={guard(media.cancelUpload)}
            onEditSave={handleEditSave}
            onRemove={guard(media.removeAttachment)}
            onRevert={guard(media.revertAttachment)}
            originalUrlByUrl={media.originalUrlByUrl}
            uploadingCount={media.uploadingCount}
            uploadingPreviews={media.uploadingPreviews}
          />
        </div>
      )}
    </>
  );
}
