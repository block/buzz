import * as React from "react";
import { ImageOff, LoaderCircle, X } from "lucide-react";

import { getRelayHttpUrl, uploadMediaBytes } from "@/shared/api/tauri";
import { extractSupportedLinkPreviews } from "@/shared/lib/linkPreview";
import {
  buildLinkPreviewSnapshotTag,
  isValidLinkPreviewSnapshotCanonicalUrl,
} from "@/shared/lib/linkPreviewSnapshot";
import {
  beginRelayOriginFetch,
  getCachedRelayOrigin,
} from "@/shared/lib/mediaUrl";
import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import { useResolvedLinkPreviews } from "@/shared/lib/useResolvedLinkPreviews";
import {
  Attachment,
  AttachmentContent,
  AttachmentDescription,
  AttachmentGroup,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from "@/shared/ui/attachment";
import { Button } from "@/shared/ui/button";

function previewHostname(href: string): string {
  try {
    return new URL(href).hostname.replace(/^www\./, "");
  } catch {
    return href;
  }
}

function ComposerLinkPreviewCard({
  preview,
}: {
  preview: ResolvedLinkPreview;
}) {
  const imageSrc = preview.imageState === "image" ? preview.imageDataUrl : null;
  const [failedImageSrc, setFailedImageSrc] = React.useState<string | null>(
    null,
  );
  const showImage = Boolean(imageSrc && failedImageSrc !== imageSrc);
  const hostname = previewHostname(preview.href);
  let path = "";
  try {
    const url = new URL(preview.href);
    path = `${url.pathname}${url.search}`;
  } catch {}

  return (
    <Attachment
      className="h-[55px] w-80 max-w-full gap-2 overflow-hidden p-0 pr-2 shadow-none"
      data-image-state={preview.imageState}
      data-link-preview={preview.kind}
      data-link-preview-composer-card=""
      state={preview.snapshotReady ? "done" : "processing"}
    >
      <AttachmentMedia
        className="h-[55px] w-[55px] rounded-none rounded-l-2xl bg-muted"
        data-link-preview-thumbnail=""
        variant="image"
      >
        {showImage ? (
          <img
            alt=""
            className="h-full w-full object-cover"
            onError={() => setFailedImageSrc(imageSrc ?? null)}
            src={imageSrc ?? undefined}
          />
        ) : preview.imageState === "pending" ? (
          <LoaderCircle
            aria-label="Loading link preview"
            className="size-4 animate-spin"
          />
        ) : preview.faviconDataUrl ? (
          <img
            alt=""
            className="size-7 rounded-md object-contain opacity-70"
            src={preview.faviconDataUrl}
          />
        ) : (
          <ImageOff aria-hidden="true" className="size-4 opacity-60" />
        )}
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle className="line-clamp-1" data-link-preview-hostname="">
          {preview.snapshotReady ? preview.title : hostname}
        </AttachmentTitle>
        <AttachmentDescription>
          {preview.snapshotReady
            ? preview.provider || hostname
            : path && path !== "/"
              ? path
              : preview.typeLabel}
        </AttachmentDescription>
      </AttachmentContent>
      <AttachmentTrigger asChild>
        <a
          aria-label={`Open ${preview.title}`}
          href={preview.href}
          rel="noreferrer"
          target="_blank"
        >
          <span className="sr-only">Open {preview.title}</span>
        </a>
      </AttachmentTrigger>
    </Attachment>
  );
}

function dataUrlBytes(dataUrl: string): Uint8Array | null {
  const match = /^data:([^;,]+);base64,([A-Za-z0-9+/=]+)$/.exec(dataUrl);
  if (!match) return null;
  try {
    return Uint8Array.from(atob(match[2]), (char) => char.charCodeAt(0));
  } catch {
    return null;
  }
}

async function uploadDataUrl(
  dataUrl: string | null | undefined,
  filename: string,
) {
  if (!dataUrl) return { url: "", sha256: "" };
  const bytes = dataUrlBytes(dataUrl);
  if (!bytes) throw new Error("invalid preview media data");
  const uploaded = await uploadMediaBytes([...bytes], filename);
  return { url: uploaded.url, sha256: uploaded.sha256 };
}

export function useComposerLinkPreviews(content: string) {
  const [suppressed, setSuppressed] = React.useState(false);
  const candidates = React.useMemo(
    () =>
      extractSupportedLinkPreviews(content).filter((preview) =>
        preview.href.startsWith("buzz://")
          ? true
          : isValidLinkPreviewSnapshotCanonicalUrl(preview.href),
      ),
    [content],
  );
  const previews = useResolvedLinkPreviews(suppressed ? [] : candidates);
  React.useEffect(() => {
    if (candidates.length === 0) setSuppressed(false);
  }, [candidates.length]);
  const [readyTags, setReadyTags] = React.useState<Record<string, string[]>>(
    {},
  );
  const readyTagsRef = React.useRef<string[][]>([]);
  const readyTagsByHrefRef = React.useRef(readyTags);
  readyTagsByHrefRef.current = readyTags;
  const suppressedRef = React.useRef(suppressed);
  suppressedRef.current = suppressed;
  const uploadsRef = React.useRef(new Set<string>());
  const activeHrefsRef = React.useRef(new Set<string>());
  activeHrefsRef.current = new Set(candidates.map((preview) => preview.href));

  React.useEffect(() => {
    if (getCachedRelayOrigin()) return;
    const publishRelayOrigin = beginRelayOriginFetch();
    void getRelayHttpUrl()
      .then((url) => publishRelayOrigin(url))
      .catch(() => publishRelayOrigin(null));
  }, []);

  React.useEffect(() => {
    const active = new Set(candidates.map((preview) => preview.href));
    setReadyTags((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([href]) => active.has(href)),
      ),
    );
  }, [candidates]);

  React.useEffect(() => {
    for (const preview of previews) {
      if (
        !preview.snapshotReady ||
        readyTags[preview.href] ||
        uploadsRef.current.has(preview.href)
      )
        continue;
      uploadsRef.current.add(preview.href);
      void Promise.all([
        uploadDataUrl(preview.imageDataUrl, "link-preview-image.png"),
        uploadDataUrl(preview.faviconDataUrl, "link-preview-favicon.png"),
      ])
        .then(([image, favicon]) => {
          if (!activeHrefsRef.current.has(preview.href)) return;
          const tag = buildLinkPreviewSnapshotTag({
            canonicalUrl: preview.href,
            title: preview.title,
            siteName: preview.provider,
            description: preview.description ?? "",
            imageUrl: image.url,
            imageSha256: image.sha256,
            faviconUrl: favicon.url,
            faviconSha256: favicon.sha256,
          });
          if (!tag) return;
          setReadyTags((current) => ({ ...current, [preview.href]: tag }));
        })
        .catch(() => {})
        .finally(() => uploadsRef.current.delete(preview.href));
    }
  }, [previews, readyTags]);

  readyTagsRef.current = suppressed
    ? [["link-preview", "none"]]
    : candidates.flatMap((candidate) =>
        readyTags[candidate.href] ? [readyTags[candidate.href]] : [],
      );
  const hideAll = React.useCallback(() => setSuppressed(true), []);
  const previewList = previews.length ? (
    <div
      className="mb-2"
      data-composer-link-previews=""
      data-ready-snapshot-count={readyTagsRef.current.length}
    >
      <div className="flex max-w-full items-start gap-1">
        <AttachmentGroup className="max-w-full flex-row flex-wrap items-start overflow-visible pb-0">
          {previews.map((preview) => (
            <ComposerLinkPreviewCard key={preview.href} preview={preview} />
          ))}
        </AttachmentGroup>
        <Button
          aria-label="Hide all link previews"
          className="mt-1 size-5 shrink-0 rounded-full text-muted-foreground hover:text-foreground [&_svg]:size-3"
          data-testid="composer-hide-link-previews"
          onClick={hideAll}
          size="icon-xs"
          title="Hide previews"
          type="button"
          variant="ghost"
        >
          <X aria-hidden="true" />
        </Button>
      </div>
    </div>
  ) : null;
  const getReadyTags = React.useCallback(() => {
    if (suppressedRef.current) return [["link-preview", "none"]];
    return [...activeHrefsRef.current].flatMap((href) => {
      const tag = readyTagsByHrefRef.current[href];
      return tag ? [tag] : [];
    });
  }, []);
  return { previewList, getReadyTags };
}
