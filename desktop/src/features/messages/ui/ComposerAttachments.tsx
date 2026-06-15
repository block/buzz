import * as React from "react";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { FileText, Play, X } from "lucide-react";

import type { BlobDescriptor } from "@/shared/api/tauri";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import {
  shortHash,
  type UploadingAttachmentPreview,
} from "@/features/messages/lib/useMediaUpload";
import { cn } from "@/shared/lib/cn";
import { Progress } from "@/shared/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/** Dashed-border overlay shown when a file is dragged over the composer form. */
export function DropZoneOverlay({ className }: { className?: string }) {
  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary bg-primary/10",
        className,
      )}
    >
      <span className="text-sm font-medium text-primary">
        Drop files to upload
      </span>
    </div>
  );
}

type ComposerAttachmentsProps = {
  attachments: BlobDescriptor[];
  isUploading?: boolean;
  onCancelUpload?: (previewId: number) => void;
  uploadingCount?: number;
  uploadingPreviews?: UploadingAttachmentPreview[];
  onRemove: (url: string) => void;
};

const COMPOSER_MEDIA_HEIGHT_PX = 55;
const COMPOSER_MEDIA_MAX_WIDTH_PX = 129;
const COMPOSER_MEDIA_MIN_WIDTH_PX = 64;

function aspectRatioFromDim(dim?: string): number | undefined {
  if (!dim) return undefined;
  const match = dim.match(/^(\d+)x(\d+)$/i);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (!Number.isFinite(width) || !Number.isFinite(height) || height <= 0) {
    return undefined;
  }
  return width / height;
}

function composerMediaStyle(dim?: string): React.CSSProperties {
  const aspectRatio = aspectRatioFromDim(dim) ?? 16 / 9;
  const widthPx = Math.round(
    Math.min(
      COMPOSER_MEDIA_MAX_WIDTH_PX,
      Math.max(
        COMPOSER_MEDIA_MIN_WIDTH_PX,
        aspectRatio * COMPOSER_MEDIA_HEIGHT_PX,
      ),
    ),
  );
  return {
    aspectRatio: String(aspectRatio),
    height: COMPOSER_MEDIA_HEIGHT_PX,
    width: widthPx,
  };
}

/**
 * Thumbnail previews for uploaded attachments in the composer.
 * Each attachment shows as a small image with a remove button and
 * a short hash label (e.g. "a3f2").
 */
export const ComposerAttachments = React.memo(function ComposerAttachments({
  attachments,
  isUploading = false,
  uploadingCount = 0,
  uploadingPreviews = [],
  onCancelUpload,
  onRemove,
}: ComposerAttachmentsProps) {
  if (attachments.length === 0 && !isUploading) return null;

  const uploadPlaceholders: UploadingAttachmentPreview[] =
    uploadingPreviews.length > 0
      ? uploadingPreviews
      : Array.from({ length: uploadingCount || 1 }, (_, index) => ({
          id: -index - 1,
        }));

  return (
    <LayoutGroup>
      <motion.div
        layout
        className="flex items-center gap-2"
        transition={{ type: "spring", stiffness: 500, damping: 30 }}
      >
        <AnimatePresence mode="popLayout">
          {attachments.map((attachment) => {
            const hash = shortHash(attachment.sha256);
            const isVideo = attachment.type.startsWith("video/");
            const isImage = attachment.type.startsWith("image/");
            const isFile = !isVideo && !isImage;
            const thumbUrl = attachment.thumb
              ? rewriteRelayUrl(attachment.thumb)
              : rewriteRelayUrl(attachment.url);
            const videoPosterUrl = attachment.image
              ? rewriteRelayUrl(attachment.image)
              : attachment.thumb
                ? rewriteRelayUrl(attachment.thumb)
                : undefined;
            const mediaStyle = composerMediaStyle(attachment.dim);

            // Generic file: compact chip with a file icon + filename, plus the
            // same remove button. No lightbox (nothing to preview).
            if (isFile) {
              const label =
                attachment.filename ||
                attachment.url.split("/").pop() ||
                `file ${hash}`;
              return (
                <motion.div
                  key={attachment.url}
                  layout
                  initial={false}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.8 }}
                  transition={{ type: "spring", stiffness: 500, damping: 30 }}
                  className="group relative"
                >
                  <div className="flex h-5 max-w-[10rem] items-center gap-1 rounded border border-border/70 bg-muted px-1.5">
                    <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="truncate text-[10px] text-muted-foreground">
                      {label}
                    </span>
                  </div>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={() => onRemove(attachment.url)}
                        className="absolute -right-1 -top-1 hidden h-4 w-4 items-center justify-center rounded-full bg-foreground text-background group-hover:flex"
                      >
                        <X className="h-2.5 w-2.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent>Remove attachment</TooltipContent>
                  </Tooltip>
                </motion.div>
              );
            }

            return (
              <motion.div
                key={attachment.url}
                layout
                initial={false}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.8 }}
                transition={{ type: "spring", stiffness: 500, damping: 30 }}
                className="group relative"
              >
                <div
                  className="relative h-[55px] max-w-[129px]"
                  style={mediaStyle}
                >
                  <DialogPrimitive.Root>
                    <DialogPrimitive.Trigger asChild>
                      <div className="h-full w-full cursor-pointer overflow-hidden rounded-2xl border border-border/70">
                        {isVideo ? (
                          <div className="relative flex h-full w-full items-center justify-center bg-muted text-white">
                            {videoPosterUrl ? (
                              <img
                                src={videoPosterUrl}
                                alt={`Video attachment ${hash}`}
                                className="h-full w-full object-cover"
                              />
                            ) : (
                              <div className="h-full w-full bg-muted/80" />
                            )}
                            <div className="absolute inset-0 bg-black/15" />
                            <div className="absolute flex h-5 w-5 items-center justify-center rounded-full bg-black/55 backdrop-blur-sm">
                              <Play className="h-4 w-4 fill-white text-white" />
                            </div>
                          </div>
                        ) : (
                          <img
                            src={thumbUrl}
                            alt={`Attachment ${hash}`}
                            className="h-full w-full object-contain"
                          />
                        )}
                      </div>
                    </DialogPrimitive.Trigger>
                    <DialogPrimitive.Portal>
                      <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
                      <DialogPrimitive.Content
                        className="fixed inset-0 z-50 flex items-center justify-center p-8"
                        onPointerDownOutside={(e) => e.preventDefault()}
                        onInteractOutside={(e) => e.preventDefault()}
                      >
                        <DialogPrimitive.Title className="sr-only">
                          Attachment {hash} preview
                        </DialogPrimitive.Title>
                        <DialogPrimitive.Description className="sr-only">
                          Full-size attachment preview. Press Escape or click
                          outside to close.
                        </DialogPrimitive.Description>
                        <DialogPrimitive.Close
                          className="absolute inset-0 cursor-default"
                          aria-label="Close lightbox"
                        />
                        {isVideo ? (
                          // biome-ignore lint/a11y/useMediaCaption: user-uploaded video, no captions available
                          <video
                            src={rewriteRelayUrl(attachment.url)}
                            controls
                            className="relative max-h-[90vh] max-w-[90vw] rounded-lg"
                          />
                        ) : (
                          <img
                            alt={`Attachment ${hash}`}
                            className="relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain"
                            src={rewriteRelayUrl(attachment.url)}
                          />
                        )}
                        <DialogPrimitive.Close className="absolute right-4 top-4 rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-hidden focus:ring-2 focus:ring-white/30">
                          <X className="h-4 w-4" />
                          <span className="sr-only">Close</span>
                        </DialogPrimitive.Close>
                      </DialogPrimitive.Content>
                    </DialogPrimitive.Portal>
                  </DialogPrimitive.Root>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={() => onRemove(attachment.url)}
                        className="absolute -right-1 -top-1 hidden h-4 w-4 items-center justify-center rounded-full bg-foreground text-background group-hover:flex"
                      >
                        <X className="h-2.5 w-2.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent>Remove attachment</TooltipContent>
                  </Tooltip>
                </div>
              </motion.div>
            );
          })}
          {isUploading &&
            uploadPlaceholders.map((preview) => (
              <motion.div
                key={`upload-placeholder-${preview.id}`}
                layout
                initial={{ opacity: 0, scale: 0.8 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.8 }}
                transition={{ type: "spring", stiffness: 500, damping: 30 }}
                className="group relative"
              >
                <div
                  className="relative h-[55px] max-w-[129px]"
                  style={composerMediaStyle(preview.dim)}
                >
                  <div className="h-full w-full overflow-hidden rounded-2xl border border-border/70 bg-muted">
                    {preview.posterUrl ? (
                      <img
                        src={preview.posterUrl}
                        alt={`Uploading ${preview.filename ?? "video"}`}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <div className="h-full w-full animate-pulse bg-muted" />
                    )}
                    <div className="absolute inset-0 flex items-end rounded-2xl bg-background/25 px-2 pb-1.5">
                      <Progress
                        aria-label={`Uploading ${preview.filename ?? "attachment"}`}
                        className={cn(
                          "h-1",
                          preview.posterUrl
                            ? "bg-white/30 [&>div]:bg-white"
                            : "bg-foreground/15 [&>div]:bg-foreground/80",
                        )}
                        data-testid="upload-progress"
                        value={preview.progress ?? null}
                      />
                    </div>
                  </div>
                  {onCancelUpload && preview.id >= 0 ? (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          aria-label="Cancel upload"
                          onClick={() => onCancelUpload(preview.id)}
                          className="absolute -right-1 -top-1 z-10 flex h-4 w-4 items-center justify-center rounded-full bg-foreground text-background"
                        >
                          <X className="h-2.5 w-2.5" />
                        </button>
                      </TooltipTrigger>
                      <TooltipContent>Cancel upload</TooltipContent>
                    </Tooltip>
                  ) : null}
                </div>
              </motion.div>
            ))}
        </AnimatePresence>
      </motion.div>
    </LayoutGroup>
  );
});
