import { ImageOff } from "lucide-react";
import { useState } from "react";

import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import { cn } from "@/shared/lib/cn";
import {
  Attachment,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from "@/shared/ui/attachment";
import { LinkPreviewControls } from "@/shared/ui/link-preview-controls";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";

function getHostname(preview: ResolvedLinkPreview): string {
  if (preview.href.startsWith("buzz://")) return preview.provider;
  try {
    return new URL(preview.href).hostname.replace(/^www\./, "");
  } catch {
    return preview.provider;
  }
}

function LinkPreviewImageFallback({
  preview,
}: {
  preview: ResolvedLinkPreview;
}) {
  return (
    <div
      aria-hidden="true"
      className="flex h-full w-full items-center justify-center bg-muted text-muted-foreground"
      data-link-preview-image-fallback=""
    >
      {preview.faviconDataUrl ? (
        <img
          alt=""
          className="size-7 rounded-md object-contain opacity-70"
          src={preview.faviconDataUrl}
        />
      ) : (
        <ImageOff className="size-5 opacity-60" />
      )}
    </div>
  );
}

export function CompactLinkPreviewAttachment({
  className,
  onOpen,
  onRemove,
  preview,
  showControls = false,
}: {
  className?: string;
  onOpen?: () => void;
  onRemove?: () => void;
  preview: ResolvedLinkPreview;
  showControls?: boolean;
}) {
  const reserveImage = preview.imageState !== "none";
  const imageSrc =
    preview.imageState === "image" ? (preview.imageDataUrl ?? null) : null;
  const [failedImageSrc, setFailedImageSrc] = useState<string | null>(null);
  const showImage = Boolean(imageSrc && failedImageSrc !== imageSrc);
  const showFallback =
    preview.imageState === "fallback" || Boolean(imageSrc && !showImage);
  const hostname = getHostname(preview);
  const showBuzzMark =
    preview.kind === "buzz-pull-request" ||
    preview.kind === "buzz-issue" ||
    preview.kind === "buzz-repository";

  return (
    <div className={cn("relative w-96 max-w-full shrink-0", className)}>
      <Attachment
        className={cn(
          "w-full bg-transparent no-underline shadow-none hover:bg-transparent",
          reserveImage
            ? "h-21 min-h-21 max-h-21 gap-0 border-0 p-0 hover:border-transparent"
            : "rounded-none border-0 border-l-[3px] border-border px-0 py-1 pl-3 hover:border-border",
        )}
        data-image-state={preview.imageState}
        data-link-preview={preview.kind}
        orientation="horizontal"
      >
        {reserveImage ? (
          <AttachmentMedia
            aria-hidden={showImage ? undefined : "true"}
            className="aspect-auto h-full min-h-0 w-30 min-w-30 max-w-30 self-stretch rounded-xl bg-muted sm:w-34 sm:min-w-34 sm:max-w-34"
            data-link-preview-thumbnail=""
            variant="image"
          >
            {showImage ? (
              <img
                alt={`Preview from ${preview.imageDomain}`}
                className="h-full w-full object-cover"
                onError={() => setFailedImageSrc(imageSrc)}
                src={imageSrc ?? undefined}
              />
            ) : showFallback ? (
              <LinkPreviewImageFallback preview={preview} />
            ) : (
              <div
                className="h-full w-full animate-pulse bg-muted-foreground/10"
                data-link-preview-skeleton=""
              />
            )}
          </AttachmentMedia>
        ) : null}
        <AttachmentContent className={reserveImage ? "px-2 py-2" : undefined}>
          <a
            className="relative z-20 flex w-fit max-w-full min-w-0 items-center gap-1.5 text-xs font-normal leading-4 text-muted-foreground/70 group-hover/attachment:underline"
            data-link-preview-hostname=""
            href={preview.href}
            onClick={
              onOpen
                ? (event) => {
                    event.preventDefault();
                    onOpen();
                  }
                : undefined
            }
            rel="noreferrer"
            target="_blank"
          >
            {showBuzzMark ? (
              <span
                aria-hidden="true"
                className="flex size-3 shrink-0 items-center text-foreground/70"
                data-link-preview-hostname-buzz-mark=""
              >
                <BuzzMark className="h-auto w-full" />
              </span>
            ) : preview.faviconDataUrl ? (
              <img
                alt=""
                aria-hidden="true"
                className="size-3 shrink-0 rounded-sm object-contain"
                data-link-preview-hostname-favicon=""
                src={preview.faviconDataUrl}
              />
            ) : null}
            <span className="truncate">{hostname}</span>
          </a>
          <AttachmentTitle className="line-clamp-2 whitespace-normal group-hover/attachment:underline">
            {preview.title}
          </AttachmentTitle>
          {preview.description ? (
            <AttachmentDescription className="text-muted-foreground/70">
              {preview.description}
            </AttachmentDescription>
          ) : null}
        </AttachmentContent>
        {onOpen ? (
          <AttachmentTrigger
            aria-label={`Open ${preview.provider} ${preview.typeLabel}: ${preview.title}`}
            className={reserveImage ? undefined : "rounded-none"}
            onClick={onOpen}
          >
            <span className="sr-only">
              Open {preview.provider} {preview.typeLabel}: {preview.title}
            </span>
          </AttachmentTrigger>
        ) : (
          <AttachmentTrigger
            asChild
            className={reserveImage ? undefined : "rounded-none"}
          >
            <a
              aria-label={`Open ${preview.provider} ${preview.typeLabel}: ${preview.title}`}
              href={preview.href}
              rel="noreferrer"
              target="_blank"
            >
              <span className="sr-only">
                Open {preview.provider} {preview.typeLabel}: {preview.title}
              </span>
            </a>
          </AttachmentTrigger>
        )}
      </Attachment>
      {showControls ? (
        <LinkPreviewControls onRemove={onRemove} placement="left" />
      ) : null}
    </div>
  );
}
