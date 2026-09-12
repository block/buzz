import { ChevronDown, ChevronUp, ImageOff } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useState } from "react";

import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import { cn } from "@/shared/lib/cn";
import { LinkPreviewControls } from "@/shared/ui/link-preview-controls";

export type LinkPreviewImageLightboxProps = {
  alt: string;
  children: ReactNode;
  className?: string;
  src: string;
};

export type LinkPreviewImageLightboxComponent =
  ComponentType<LinkPreviewImageLightboxProps>;

function LinkPreviewIdentity({ preview }: { preview: ResolvedLinkPreview }) {
  if (preview.faviconDataUrl) {
    return (
      <img
        alt=""
        aria-hidden="true"
        className="size-4 rounded-sm object-contain"
        data-link-preview-favicon=""
        src={preview.faviconDataUrl}
      />
    );
  }
  return null;
}

function getHostname(preview: ResolvedLinkPreview): string {
  try {
    return new URL(preview.href).hostname.replace(/^www\./, "");
  } catch {
    return preview.provider;
  }
}

function isTweetPreview(preview: ResolvedLinkPreview): boolean {
  try {
    const url = new URL(preview.href);
    return (
      (url.hostname === "x.com" || url.hostname === "twitter.com") &&
      /^\/[^/]+\/status\/\d+/.test(url.pathname)
    );
  } catch {
    return false;
  }
}

function LinkPreviewImage({
  aspectClassName,
  className,
  ImageLightbox,
  preview,
}: {
  aspectClassName: string;
  className?: string;
  ImageLightbox: LinkPreviewImageLightboxComponent;
  preview: ResolvedLinkPreview;
}) {
  const imageSrc =
    preview.imageState === "image" ? preview.imageDataUrl : undefined;
  const [failedImageSrc, setFailedImageSrc] = useState<string | null>(null);
  const imageFailed = Boolean(imageSrc && failedImageSrc === imageSrc);
  const showFallback = preview.imageState === "fallback" || imageFailed;
  const alt = `Preview from ${preview.imageDomain}`;

  if (!imageSrc || imageFailed) {
    return (
      <div
        className={cn("overflow-hidden rounded-xl bg-muted", className)}
        data-link-preview-thumbnail=""
      >
        {showFallback ? (
          <div
            aria-hidden="true"
            className={cn(
              "flex w-full items-center justify-center bg-muted text-muted-foreground",
              aspectClassName,
            )}
            data-link-preview-image-fallback=""
          >
            {preview.faviconDataUrl ? (
              <img
                alt=""
                className="size-10 rounded-lg object-contain opacity-70"
                src={preview.faviconDataUrl}
              />
            ) : (
              <ImageOff className="size-7 opacity-60" />
            )}
          </div>
        ) : (
          <div
            className={cn(
              "w-full animate-pulse bg-muted-foreground/10",
              aspectClassName,
            )}
            data-link-preview-skeleton=""
          />
        )}
      </div>
    );
  }

  return (
    <ImageLightbox
      alt={alt}
      className={cn("block overflow-hidden rounded-xl bg-muted", className)}
      src={imageSrc}
    >
      <div
        className={cn("w-full", aspectClassName)}
        data-link-preview-thumbnail=""
      >
        <img
          alt={alt}
          className="h-full w-full rounded-xl object-cover"
          onError={() => setFailedImageSrc(imageSrc)}
          src={imageSrc}
        />
      </div>
    </ImageLightbox>
  );
}

function LinkPreviewDescription({
  className,
  description,
}: {
  className?: string;
  description: string;
}) {
  return (
    <div
      className={cn("mt-1 space-y-2 text-sm leading-5", className)}
      data-slot="attachment-description"
    >
      {description.split(/\n{2,}/).map((paragraph) => (
        <p className="whitespace-pre-line" key={paragraph}>
          {paragraph}
        </p>
      ))}
    </div>
  );
}

function TweetPreview({
  className,
  ImageLightbox,
  onRemove,
  preview,
  showControls,
  showExpandControl,
}: {
  className?: string;
  ImageLightbox: LinkPreviewImageLightboxComponent;
  onRemove?: () => void;
  preview: ResolvedLinkPreview;
  showControls: boolean;
  showExpandControl: boolean;
}) {
  const [contentExpanded, setContentExpanded] = useState(true);
  const reserveImage = preview.imageState !== "none";
  const hasExpandableContent = Boolean(preview.description) || reserveImage;
  const hostname = getHostname(preview);

  return (
    <div
      className={cn(
        "relative w-lg max-w-full shrink-0 border-l-[3px] border-border py-1 pl-3",
        className,
      )}
      data-image-state={preview.imageState}
      data-link-preview={preview.kind}
      data-tweet-preview=""
    >
      <a
        className="block w-fit max-w-full truncate text-xs leading-4 text-muted-foreground/70 hover:underline"
        data-link-preview-hostname=""
        href={preview.href}
        rel="noreferrer"
        target="_blank"
      >
        {hostname}
      </a>
      <a
        className="mt-0.5 line-clamp-2 whitespace-normal text-sm font-semibold leading-5 text-foreground hover:underline"
        href={preview.href}
        rel="noreferrer"
        target="_blank"
      >
        {preview.title}
      </a>
      {contentExpanded && preview.description ? (
        <LinkPreviewDescription
          className="text-foreground"
          description={preview.description}
        />
      ) : null}
      {contentExpanded && reserveImage ? (
        <LinkPreviewImage
          aspectClassName="aspect-video"
          className="mt-2 w-full max-w-96"
          ImageLightbox={ImageLightbox}
          preview={preview}
        />
      ) : null}
      {showExpandControl && hasExpandableContent ? (
        <button
          aria-expanded={contentExpanded}
          className="mt-1 flex items-center gap-1 text-xs leading-4 text-muted-foreground hover:text-foreground"
          onClick={() => setContentExpanded((expanded) => !expanded)}
          type="button"
        >
          {contentExpanded ? (
            <ChevronUp aria-hidden="true" className="size-3" />
          ) : (
            <ChevronDown aria-hidden="true" className="size-3" />
          )}
          {contentExpanded ? "Show less" : "Show more"}
        </button>
      ) : null}
      {showControls ? (
        <LinkPreviewControls onRemove={onRemove} placement="left" />
      ) : null}
    </div>
  );
}

export function RichLinkPreviewAttachment({
  className,
  ImageLightbox,
  onOpen,
  onRemove,
  preview,
  showControls = false,
  showExpandControl = true,
}: {
  className?: string;
  ImageLightbox: LinkPreviewImageLightboxComponent;
  onOpen?: () => void;
  onRemove?: () => void;
  preview: ResolvedLinkPreview;
  showControls?: boolean;
  showExpandControl?: boolean;
}) {
  const [contentExpanded, setContentExpanded] = useState(true);

  if (isTweetPreview(preview)) {
    return (
      <TweetPreview
        className={className}
        ImageLightbox={ImageLightbox}
        onRemove={onRemove}
        preview={preview}
        showControls={showControls}
        showExpandControl={showExpandControl}
      />
    );
  }

  const reserveImage = preview.imageState !== "none";
  const hasExpandableContent = Boolean(preview.description) || reserveImage;
  const hostname = getHostname(preview);

  return (
    <div
      className={cn(
        "relative w-lg max-w-full shrink-0 border-l-[3px] border-border py-1 pl-3",
        className,
      )}
      data-image-state={preview.imageState}
      data-link-preview={preview.kind}
      data-link-preview-inline=""
    >
      <div
        className={cn(contentExpanded && reserveImage && "min-h-[3.875rem]")}
      >
        <a
          className="flex w-fit max-w-full items-center gap-1.5 text-xs leading-4 text-muted-foreground/70 hover:underline"
          data-link-preview-identity=""
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
          <LinkPreviewIdentity preview={preview} />
          <span className="truncate">{hostname}</span>
        </a>
        <a
          aria-label={`Open ${preview.provider} ${preview.typeLabel}: ${preview.title}`}
          className="mt-0.5 block text-sm font-semibold leading-5 text-foreground hover:underline"
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
          <span className="line-clamp-2">{preview.title}</span>
        </a>
        {contentExpanded && preview.description ? (
          <LinkPreviewDescription
            className="text-muted-foreground"
            description={preview.description}
          />
        ) : null}
      </div>
      {contentExpanded && reserveImage ? (
        <LinkPreviewImage
          aspectClassName="aspect-[1.91/1]"
          className="mt-2 w-full max-w-96"
          ImageLightbox={ImageLightbox}
          preview={preview}
        />
      ) : null}
      {showExpandControl && hasExpandableContent ? (
        <button
          aria-expanded={contentExpanded}
          className="mt-1 flex items-center gap-1 text-xs leading-4 text-muted-foreground hover:text-foreground"
          onClick={() => setContentExpanded((expanded) => !expanded)}
          type="button"
        >
          {contentExpanded ? (
            <ChevronUp aria-hidden="true" className="size-3" />
          ) : (
            <ChevronDown aria-hidden="true" className="size-3" />
          )}
          {contentExpanded ? "Show less" : "Show more"}
        </button>
      ) : null}
      {showControls ? (
        <LinkPreviewControls onRemove={onRemove} placement="left" />
      ) : null}
    </div>
  );
}
