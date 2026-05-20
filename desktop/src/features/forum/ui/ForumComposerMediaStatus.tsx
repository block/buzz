import type { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { ComposerAttachments } from "@/features/messages/ui/ComposerAttachments";

type ComposerMedia = Pick<
  ReturnType<typeof useMediaUpload>,
  | "isUploading"
  | "pendingImeta"
  | "removeAttachment"
  | "setUploadState"
  | "uploadState"
  | "uploadingCount"
>;

type ForumComposerMediaStatusProps = {
  media: ComposerMedia;
};

export function ForumComposerMediaStatus({
  media,
}: ForumComposerMediaStatusProps) {
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
            onRemove={media.removeAttachment}
            uploadingCount={media.uploadingCount}
          />
        </div>
      )}
    </>
  );
}
