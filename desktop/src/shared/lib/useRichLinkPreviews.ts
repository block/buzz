import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";

import type { RichLinkPreviewMetadata } from "./linkPreview";

type CacheEntry =
  | Promise<RichLinkPreviewMetadata | null>
  | RichLinkPreviewMetadata
  | null;

const metadataCache = new Map<string, CacheEntry>();

function fetchMetadata(href: string): Promise<RichLinkPreviewMetadata | null> {
  return invokeTauri<RichLinkPreviewMetadata | null>(
    "fetch_link_preview_metadata",
    { href },
  );
}

function cacheMetadata(href: string): Promise<RichLinkPreviewMetadata | null> {
  const cached = metadataCache.get(href);
  if (cached instanceof Promise) return cached;
  if (cached !== undefined) return Promise.resolve(cached);

  const promise = fetchMetadata(href)
    .then((metadata) => {
      metadataCache.set(href, metadata);
      return metadata;
    })
    .catch(() => {
      metadataCache.set(href, null);
      return null;
    });
  metadataCache.set(href, promise);
  return promise;
}

function isRenderable(metadata: RichLinkPreviewMetadata | null): boolean {
  return metadata !== null && (metadata.title ?? "").trim() !== "";
}

/**
 * Resolve rich OpenGraph metadata for generic message URLs. Only URLs whose
 * fetch produced a usable title are returned, so cards never render empty.
 */
export function useRichLinkPreviews(urls: string[]): RichLinkPreviewMetadata[] {
  const [resolved, setResolved] = React.useState<
    Record<string, RichLinkPreviewMetadata>
  >({});

  React.useEffect(() => {
    if (urls.length === 0) return undefined;
    let cancelled = false;

    for (const href of urls) {
      const cached = metadataCache.get(href);
      if (cached !== undefined && !(cached instanceof Promise)) {
        if (isRenderable(cached)) {
          const metadata = cached as RichLinkPreviewMetadata;
          setResolved((current) =>
            current[href] === metadata
              ? current
              : { ...current, [href]: metadata },
          );
        }
        continue;
      }

      void cacheMetadata(href).then((metadata) => {
        if (cancelled || !isRenderable(metadata)) return;
        const value = metadata as RichLinkPreviewMetadata;
        setResolved((current) =>
          current[href] === value ? current : { ...current, [href]: value },
        );
      });
    }

    return () => {
      cancelled = true;
    };
  }, [urls]);

  return React.useMemo(
    () =>
      urls.flatMap((href) => {
        const metadata = resolved[href];
        return metadata ? [metadata] : [];
      }),
    [urls, resolved],
  );
}
