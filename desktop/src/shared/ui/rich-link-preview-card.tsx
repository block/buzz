import * as React from "react";
import { Globe } from "lucide-react";

import {
  extractGenericLinkPreviewUrls,
  type RichLinkPreviewMetadata,
} from "@/shared/lib/linkPreview";
import { useRichLinkPreviews } from "@/shared/lib/useRichLinkPreviews";
import { cn } from "@/shared/lib/cn";
import { AttachmentGroup } from "@/shared/ui/attachment";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

function hostnameOf(href: string): string {
  try {
    return new URL(href).hostname.replace(/^www\./, "");
  } catch {
    return href;
  }
}

function Favicon({ metadata }: { metadata: RichLinkPreviewMetadata }) {
  const [failed, setFailed] = React.useState(false);

  if (!metadata.faviconUrl || failed) {
    return <Globe aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />;
  }

  return (
    <img
      alt=""
      className="h-3.5 w-3.5 shrink-0 rounded-xs"
      loading="lazy"
      onError={() => setFailed(true)}
      src={metadata.faviconUrl}
    />
  );
}

function PreviewImage({ src }: { src: string }) {
  const [failed, setFailed] = React.useState(false);
  if (failed) return null;

  return (
    <img
      alt=""
      className="h-16 w-16 shrink-0 rounded-lg border border-border/50 object-cover"
      loading="lazy"
      onError={() => setFailed(true)}
      src={src}
    />
  );
}

/**
 * Extracts generic URLs from message content, resolves their OpenGraph
 * metadata, and renders a group of rich preview cards (or nothing).
 */
export function RichLinkPreviews({ content }: { content: string }) {
  const urls = React.useMemo(
    () => extractGenericLinkPreviewUrls(content),
    [content],
  );
  const previews = useRichLinkPreviews(urls);

  if (previews.length === 0) return null;

  return (
    <AttachmentGroup
      className="max-w-full flex-wrap overflow-visible pb-0"
      data-rich-link-preview-list=""
    >
      {previews.map((metadata) => (
        <RichLinkPreviewCard key={metadata.href} metadata={metadata} />
      ))}
    </AttachmentGroup>
  );
}

export function RichLinkPreviewCard({
  className,
  metadata,
}: {
  className?: string;
  metadata: RichLinkPreviewMetadata;
}) {
  const cardRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(cardRef);
  const hostname = hostnameOf(metadata.href);

  return (
    <div
      ref={cardRef}
      className={cn(
        "group/rich-preview relative w-96 max-w-full overflow-hidden rounded-2xl border border-border/70 px-3 py-2.5 transition-colors",
        "hover:border-border",
        className,
      )}
      data-rich-link-preview=""
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-xs font-medium leading-4 text-muted-foreground">
            <Favicon metadata={metadata} />
            <span className="truncate">{metadata.siteName ?? hostname}</span>
          </div>
          <div className="mt-1 truncate text-sm font-semibold leading-5 text-primary">
            {metadata.title}
          </div>
          {metadata.description ? (
            <div className="mt-0.5 line-clamp-2 text-xs leading-4 text-muted-foreground">
              {metadata.description}
            </div>
          ) : null}
        </div>
        {metadata.imageUrl ? <PreviewImage src={metadata.imageUrl} /> : null}
      </div>
      <a
        aria-label={`Open link: ${metadata.title ?? hostname}`}
        className="absolute inset-0 z-10"
        href={metadata.href}
        rel="noreferrer"
        target="_blank"
      >
        <span className="sr-only">Open link: {metadata.title ?? hostname}</span>
      </a>
    </div>
  );
}
