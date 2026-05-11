import * as React from "react";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";

import type { BlobDescriptor } from "@/shared/api/tauri";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { shortHash } from "@/features/messages/lib/useMediaUpload";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/** Dashed-border overlay shown when a file is dragged over the composer form. */
export function DropZoneOverlay() {
  return (
    <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary bg-primary/10">
      <span className="text-sm font-medium text-primary">
        Drop files to upload
      </span>
    </div>
  );
}

type ComposerAttachmentsProps = {
  attachments: BlobDescriptor[];
  isUploading?: boolean;
  uploadingCount?: number;
  onRemove: (url: string) => void;
};

/**
 * Thumbnail previews for uploaded attachments in the composer.
 * Each attachment shows as a small image with a remove button and
 * a short hash label (e.g. "a3f2").
 */
export const ComposerAttachments = React.memo(function ComposerAttachments({
  attachments,
  isUploading = false,
  uploadingCount = 0,
  onRemove,
}: ComposerAttachmentsProps) {
  if (attachments.length === 0 && !isUploading) return null;

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
            const thumbUrl = attachment.thumb
              ? rewriteRelayUrl(attachment.thumb)
              : rewriteRelayUrl(attachment.url);

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
                <div className="relative h-5 w-10">
                  <DialogPrimitive.Root>
                    <DialogPrimitive.Trigger asChild>
                      <div className="h-full w-full cursor-pointer overflow-hidden rounded border border-border/70">
                        {isVideo ? (
                          <div className="flex h-full w-full items-center justify-center bg-muted text-[10px] text-muted-foreground">
                            ▶
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
                        <DialogPrimitive.Close className="absolute right-4 top-4 rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-none focus:ring-2 focus:ring-white/30">
                          <X className="h-5 w-5" />
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
            Array.from({ length: uploadingCount || 1 }).map((_, i) => (
              <motion.div
                // biome-ignore lint/suspicious/noArrayIndexKey: placeholders have no stable identity
                key={`upload-placeholder-${i}`}
                layout
                initial={{ opacity: 0, scale: 0.8 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.8 }}
                transition={{ type: "spring", stiffness: 500, damping: 30 }}
              >
                <div className="relative h-5 w-10 overflow-hidden rounded border border-border/70">
                  <div className="h-full w-full animate-pulse bg-muted" />
                </div>
              </motion.div>
            ))}
        </AnimatePresence>
      </motion.div>
    </LayoutGroup>
  );
});
