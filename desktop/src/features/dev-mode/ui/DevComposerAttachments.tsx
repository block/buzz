import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { UploadingAttachmentPreview } from "@/features/messages/lib/useMediaUpload";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useMediaProxyPort } from "@/shared/lib/useMediaProxyPort";

/**
 * Pending and in-flight attachments for a dev-mode composer: compact,
 * square-cornered chips in the terminal aesthetic. Images show a small
 * thumbnail; videos and other files show their filename.
 */
export function DevComposerAttachments({
  pendingImeta,
  uploadingPreviews,
  errorMessage,
  onRemove,
}: {
  pendingImeta: ImetaMedia[];
  uploadingPreviews: UploadingAttachmentPreview[];
  errorMessage: string | null;
  onRemove: (url: string) => void;
}) {
  // Re-render after the async proxy-port lookup so first-paint
  // buzz-media:// fallbacks become authenticated loopback URLs.
  useMediaProxyPort();

  if (
    pendingImeta.length === 0 &&
    uploadingPreviews.length === 0 &&
    !errorMessage
  ) {
    return null;
  }
  return (
    <div
      className="flex flex-wrap items-center gap-2 px-4 pt-2 text-xs text-muted-foreground"
      data-testid="dev-mode-attachments"
    >
      {pendingImeta.map((media) => (
        <span
          key={media.url}
          className="flex items-center gap-1.5 border border-border/50 bg-muted/40 py-0.5 pr-1 pl-1.5"
          data-testid="dev-mode-attachment-chip"
        >
          {media.type.startsWith("image/") ? (
            <img
              alt={media.filename ?? "attachment"}
              className="h-8 w-8 object-cover"
              src={rewriteRelayUrl(media.url)}
            />
          ) : null}
          <span className="max-w-40 truncate">
            {media.filename ??
              (media.type.startsWith("video/") ? "video" : "file")}
          </span>
          <button
            aria-label="remove attachment"
            className="cursor-pointer px-1 text-muted-foreground hover:text-foreground"
            onClick={() => onRemove(media.url)}
            type="button"
          >
            ×
          </button>
        </span>
      ))}
      {uploadingPreviews.map((preview) => (
        <span
          key={preview.id}
          className="border border-border/50 border-dashed px-1.5 py-0.5"
          data-testid="dev-mode-attachment-uploading"
        >
          uploading {preview.filename ?? "file"}
          {typeof preview.progress === "number" ? ` ${preview.progress}%` : "…"}
        </span>
      ))}
      {errorMessage ? (
        <span className="text-destructive">{errorMessage}</span>
      ) : null}
    </div>
  );
}
