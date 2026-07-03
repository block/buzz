import * as React from "react";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import { FileText, HatGlasses, Play, X } from "lucide-react";

import type { BlobDescriptor } from "@/shared/api/tauri";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import {
  shortHash,
  type UploadingAttachmentPreview,
} from "@/features/messages/lib/useMediaUpload";
import { cn } from "@/shared/lib/cn";
import { SimpleImageLightbox } from "@/shared/ui/SimpleImageLightbox";
import { Progress } from "@/shared/ui/progress";
import { Toggle } from "@/shared/ui/toggle";
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
  onToggleSpoiler?: (url: string) => void;
  spoileredUrls?: ReadonlySet<string>;
};

const LIGHTBOX_BUTTON_CLASS =
  "rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-hidden focus:ring-2 focus:ring-white/30";

const COMPOSER_MEDIA_HEIGHT_PX = 55;
const COMPOSER_MEDIA_WIDTH_PX = 55;

function composerMediaStyle(): React.CSSProperties {
  return {
    height: COMPOSER_MEDIA_HEIGHT_PX,
    width: COMPOSER_MEDIA_WIDTH_PX,
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
  onToggleSpoiler,
  spoileredUrls,
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
            const isSpoilered = spoileredUrls?.has(attachment.url) ?? false;
            const thumbUrl = attachment.thumb
              ? rewriteRelayUrl(attachment.thumb)
              : rewriteRelayUrl(attachment.url);
            const videoPosterUrl = attachment.image
              ? rewriteRelayUrl(attachment.image)
              : attachment.thumb
                ? rewriteRelayUrl(attachment.thumb)
                : undefined;
            const mediaStyle = composerMediaStyle();

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
                    <span className="truncate text-2xs text-muted-foreground">
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
                <AttachmentMediaLightbox
                  alt={`Attachment ${hash} preview`}
                  hash={hash}
                  isSpoilered={isSpoilered}
                  isVideo={isVideo}
                  mediaStyle={mediaStyle}
                  onToggleSpoiler={onToggleSpoiler}
                  thumbUrl={thumbUrl}
                  url={attachment.url}
                  videoPosterUrl={videoPosterUrl ?? null}
                />
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
                  className="relative h-[55px] max-w-[55px]"
                  style={composerMediaStyle()}
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

function AttachmentMediaLightbox({
  alt,
  hash,
  isSpoilered,
  isVideo,
  mediaStyle,
  onToggleSpoiler,
  thumbUrl,
  url,
  videoPosterUrl,
}: {
  alt: string;
  hash: string;
  isSpoilered: boolean;
  isVideo: boolean;
  mediaStyle: React.CSSProperties;
  onToggleSpoiler?: (url: string) => void;
  thumbUrl: string;
  url: string;
  videoPosterUrl: string | null;
}) {
  const [lightboxOpen, setLightboxOpen] = React.useState(false);
  const previewSrc = rewriteRelayUrl(url);

  return (
    <div className="relative h-[55px] max-w-[55px]" style={mediaStyle}>
      <button
        className="h-full w-full cursor-pointer overflow-hidden rounded-2xl border border-border/70"
        onClick={() => setLightboxOpen(true)}
        type="button"
      >
        {isVideo ? (
          <div className="relative flex h-full w-full items-center justify-center bg-muted text-white">
            {videoPosterUrl ? (
              <img
                alt={`Video attachment ${hash}`}
                className="h-full w-full object-cover"
                src={videoPosterUrl}
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
            alt={`Attachment ${hash}`}
            className="h-full w-full object-cover"
            src={thumbUrl}
          />
        )}
        {isSpoilered ? (
          <div
            className="pointer-events-none absolute inset-0 flex items-center justify-center rounded-2xl bg-background/55 text-foreground/70 backdrop-blur-[1px]"
            data-composer-media-spoiler=""
          >
            <HatGlasses className="h-4 w-4" />
          </div>
        ) : null}
      </button>
      <SimpleImageLightbox
        alt={alt}
        onOpenChange={setLightboxOpen}
        open={lightboxOpen}
        src={previewSrc}
        actions={
          // Hide this alongside image-edit mode once the annotation flow lands.
          onToggleSpoiler ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Toggle
                  aria-label={
                    isSpoilered ? "Remove spoiler" : "Mark as spoiler"
                  }
                  className={cn(LIGHTBOX_BUTTON_CLASS, "h-auto min-w-0")}
                  data-testid="composer-attachment-spoiler"
                  onPressedChange={() => onToggleSpoiler(url)}
                  pressed={isSpoilered}
                >
                  <HatGlasses className="h-4 w-4" />
                </Toggle>
              </TooltipTrigger>
              <TooltipContent>
                {isSpoilered ? "Remove spoiler" : "Mark as spoiler"}
              </TooltipContent>
            </Tooltip>
          ) : null
        }
      >
        {isVideo ? (
          // biome-ignore lint/a11y/useMediaCaption: user-uploaded video, no captions available
          <video
            className={cn(
              "relative max-h-[90vh] max-w-[90vw] rounded-lg",
              isSpoilered && "blur-2xl brightness-75",
            )}
            controls
            src={previewSrc}
          />
        ) : (
          <img
            alt={alt}
            className={cn(
              "relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain",
              isSpoilered && "blur-2xl brightness-75",
            )}
            src={previewSrc}
          />
        )}
        {isSpoilered ? (
          /*
           * Expanded-media counterpart of the thumbnail spoiler treatment: the
           * media itself is blurred above, and this layer centers the spoiler
           * glyph. pointer-events-none keeps controls and backdrop-close clickable.
           */
          <div
            className="pointer-events-none absolute inset-0 flex items-center justify-center text-foreground/70"
            data-lightbox-media-spoiler=""
          >
            <HatGlasses className="h-10 w-10" />
          </div>
        ) : null}
      </SimpleImageLightbox>
    </div>
  );
}
