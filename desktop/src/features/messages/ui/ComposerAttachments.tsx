import * as React from "react";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { FileText, HatGlasses, Pencil, Play, X } from "lucide-react";

import type { BlobDescriptor } from "@/shared/api/tauri";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import {
  shortHash,
  type UploadingAttachmentPreview,
} from "@/features/messages/lib/useMediaUpload";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { MODAL_BACKDROP_BLUR_CLASS } from "@/shared/ui/modalBackdrop";
import { Progress } from "@/shared/ui/progress";
import { Toggle } from "@/shared/ui/toggle";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ComposerImageEditor } from "./ComposerImageEditor";

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
  /** Upload annotated bytes as a replacement for the attachment at `url`. */
  onEditSave?: (url: string, bytes: Uint8Array) => Promise<void>;
  onRemove: (url: string) => void;
  /** Restore the pre-edit original for an annotated attachment. */
  onRevert?: (url: string) => void;
  /** Annotated attachment URL → original (pre-edit) URL. */
  originalUrlByUrl?: ReadonlyMap<string, string>;
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

type MediaAttachmentItemProps = {
  attachment: BlobDescriptor;
  isSpoilered: boolean;
  onEditSave?: (url: string, bytes: Uint8Array) => Promise<void>;
  onRemove: (url: string) => void;
  onRevert?: (url: string) => void;
  onToggleSpoiler?: (url: string) => void;
  /** Set when this attachment is an annotated replacement of an original. */
  originalUrl?: string;
};

/**
 * A single image/video attachment thumbnail with its lightbox dialog.
 * Images support an in-lightbox canvas edit mode (freehand drawing) and,
 * once annotated, an in-place revert to the original. Save closes the
 * dialog; revert keeps it open (the parent keys this item by its original
 * URL so the swap doesn't remount it).
 *
 * Forwards its ref to the root motion.div — required by the parent
 * `AnimatePresence mode="popLayout"`, which measures exiting children.
 */
const MediaAttachmentItem = React.forwardRef<
  HTMLDivElement,
  MediaAttachmentItemProps
>(function MediaAttachmentItem(
  {
    attachment,
    isSpoilered,
    onEditSave,
    onRemove,
    onRevert,
    onToggleSpoiler,
    originalUrl,
  },
  ref,
) {
  const [open, setOpen] = React.useState(false);
  const [mode, setMode] = React.useState<"view" | "edit">("view");

  const hash = shortHash(attachment.sha256);
  const isVideo = attachment.type.startsWith("video/");
  const thumbUrl = attachment.thumb
    ? rewriteRelayUrl(attachment.thumb)
    : rewriteRelayUrl(attachment.url);
  const videoPosterUrl = attachment.image
    ? rewriteRelayUrl(attachment.image)
    : attachment.thumb
      ? rewriteRelayUrl(attachment.thumb)
      : undefined;

  const canEdit = !isVideo && onEditSave !== undefined;
  const canRevert =
    !isVideo && onRevert !== undefined && originalUrl !== undefined;

  const handleOpenChange = React.useCallback((next: boolean) => {
    setOpen(next);
    if (!next) setMode("view");
  }, []);

  // Read `mode` via a ref: Radix's dismissable layer (>=1.1.14) registers a
  // stable Escape listener, so the handler would otherwise see a stale mode.
  const modeRef = React.useRef(mode);
  modeRef.current = mode;
  const handleEscapeKeyDown = React.useCallback((event: KeyboardEvent) => {
    if (modeRef.current === "edit") {
      // Escape leaves canvas mode but keeps the lightbox open.
      event.preventDefault();
      setMode("view");
    }
  }, []);

  const handleEditorSave = React.useCallback(
    async (bytes: Uint8Array) => {
      if (!onEditSave) return;
      await onEditSave(attachment.url, bytes);
      // Close on save so rapid save/redraw cycles don't orphan a blob per iteration.
      setMode("view");
      setOpen(false);
    },
    [attachment.url, onEditSave],
  );

  const handleEditorCancel = React.useCallback(() => setMode("view"), []);

  const handleRevert = React.useCallback(() => {
    onRevert?.(attachment.url);
  }, [attachment.url, onRevert]);

  return (
    <motion.div
      ref={ref}
      layout
      initial={false}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.8 }}
      transition={{ type: "spring", stiffness: 500, damping: 30 }}
      className="group relative"
    >
      <div
        className="relative h-[55px] max-w-[55px]"
        style={composerMediaStyle()}
      >
        <DialogPrimitive.Root open={open} onOpenChange={handleOpenChange}>
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
                  className="h-full w-full object-cover"
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
            </div>
          </DialogPrimitive.Trigger>
          <DialogPrimitive.Portal>
            <DialogPrimitive.Overlay
              className={cn(
                "fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0",
                MODAL_BACKDROP_BLUR_CLASS,
              )}
            />
            <DialogPrimitive.Content
              className="fixed inset-0 z-50 flex items-center justify-center p-8"
              onPointerDownOutside={(e) => e.preventDefault()}
              onInteractOutside={(e) => e.preventDefault()}
              onEscapeKeyDown={handleEscapeKeyDown}
            >
              <DialogPrimitive.Title className="sr-only">
                Attachment {hash} preview
              </DialogPrimitive.Title>
              <DialogPrimitive.Description className="sr-only">
                Full-size attachment preview. Press Escape or click outside to
                close.
              </DialogPrimitive.Description>
              {mode === "view" ? (
                <DialogPrimitive.Close
                  className="absolute inset-0 cursor-default"
                  aria-label="Close lightbox"
                />
              ) : null}
              {mode === "edit" && !isVideo ? (
                <ComposerImageEditor
                  alt={`Attachment ${hash}`}
                  src={rewriteRelayUrl(attachment.url)}
                  sourceUrl={attachment.url}
                  sourceType={attachment.type}
                  onCancel={handleEditorCancel}
                  onSave={handleEditorSave}
                />
              ) : isVideo ? (
                // biome-ignore lint/a11y/useMediaCaption: user-uploaded video, no captions available
                <video
                  src={rewriteRelayUrl(attachment.url)}
                  controls
                  className={cn(
                    "relative max-h-[90vh] max-w-[90vw] rounded-lg",
                    isSpoilered && "blur-2xl brightness-75",
                  )}
                />
              ) : (
                <img
                  alt={`Attachment ${hash}`}
                  className={cn(
                    "relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain",
                    isSpoilered && "blur-2xl brightness-75",
                  )}
                  src={rewriteRelayUrl(attachment.url)}
                />
              )}
              {mode === "view" && isSpoilered ? (
                /*
                 * Expanded-media counterpart of the thumbnail spoiler treatment:
                 * the media itself is blurred above, and this layer centers the
                 * spoiler glyph. pointer-events-none keeps controls and
                 * backdrop-close clickable.
                 */
                <div
                  className="pointer-events-none absolute inset-0 flex items-center justify-center text-foreground/70"
                  data-lightbox-media-spoiler=""
                >
                  <HatGlasses className="h-10 w-10" />
                </div>
              ) : null}
              {mode === "view" ? (
                <div className="absolute right-4 top-4 flex items-center gap-2">
                  {canRevert ? (
                    <Tooltip disableHoverableContent>
                      <TooltipTrigger asChild>
                        <Button
                          data-testid="composer-attachment-revert"
                          onClick={handleRevert}
                          size="sm"
                          type="button"
                        >
                          Revert
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>Revert to original</TooltipContent>
                    </Tooltip>
                  ) : null}
                  {onToggleSpoiler ? (
                    <Tooltip disableHoverableContent>
                      <TooltipTrigger asChild>
                        <Toggle
                          aria-label={
                            isSpoilered ? "Remove spoiler" : "Mark as spoiler"
                          }
                          className={cn(
                            LIGHTBOX_BUTTON_CLASS,
                            "h-auto min-w-0",
                          )}
                          data-testid="composer-attachment-spoiler"
                          onPressedChange={() =>
                            onToggleSpoiler(attachment.url)
                          }
                          pressed={isSpoilered}
                        >
                          <HatGlasses className="h-4 w-4" />
                        </Toggle>
                      </TooltipTrigger>
                      <TooltipContent>
                        {isSpoilered ? "Remove spoiler" : "Mark as spoiler"}
                      </TooltipContent>
                    </Tooltip>
                  ) : null}
                  {canEdit ? (
                    <Tooltip disableHoverableContent>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          className={LIGHTBOX_BUTTON_CLASS}
                          data-testid="composer-attachment-edit"
                          onClick={() => setMode("edit")}
                        >
                          <Pencil className="h-4 w-4" />
                          <span className="sr-only">Draw on image</span>
                        </button>
                      </TooltipTrigger>
                      <TooltipContent>Draw on image</TooltipContent>
                    </Tooltip>
                  ) : null}
                  <DialogPrimitive.Close className={LIGHTBOX_BUTTON_CLASS}>
                    <X className="h-4 w-4" />
                    <span className="sr-only">Close</span>
                  </DialogPrimitive.Close>
                </div>
              ) : null}
            </DialogPrimitive.Content>
          </DialogPrimitive.Portal>
        </DialogPrimitive.Root>
        <Tooltip disableHoverableContent>
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
});

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
  onEditSave,
  onRemove,
  onRevert,
  originalUrlByUrl,
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
                  <div className="flex h-5 max-w-40 items-center gap-1 rounded border border-border/70 bg-muted px-1.5">
                    <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="truncate text-2xs text-muted-foreground">
                      {label}
                    </span>
                  </div>
                  <Tooltip disableHoverableContent>
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

            const originalUrl = originalUrlByUrl?.get(attachment.url);
            return (
              <MediaAttachmentItem
                attachment={attachment}
                isSpoilered={spoileredUrls?.has(attachment.url) ?? false}
                // Annotated attachments keep their original URL as the key so
                // the in-place edit/revert URL swap doesn't remount the item
                // (which would close its open lightbox dialog).
                key={originalUrl ?? attachment.url}
                onEditSave={onEditSave}
                onRemove={onRemove}
                onRevert={onRevert}
                onToggleSpoiler={onToggleSpoiler}
                originalUrl={originalUrl}
              />
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
                    <Tooltip disableHoverableContent>
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
