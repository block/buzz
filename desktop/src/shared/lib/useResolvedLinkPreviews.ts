import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";

import type { SupportedLinkPreview } from "./linkPreview";

const GOOGLE_FALLBACK_TITLES = new Set([
  "Drive file",
  "Drive folder",
  "Document",
  "Spreadsheet",
  "Presentation",
]);

const titleCache = new Map<string, Promise<string | null> | string | null>();

function fetchLinkPreviewTitle(href: string): Promise<string | null> {
  return invokeTauri<string | null>("fetch_link_preview_title", { href });
}

function shouldResolveTitle(preview: SupportedLinkPreview): boolean {
  return (
    preview.kind.startsWith("google-") &&
    GOOGLE_FALLBACK_TITLES.has(preview.title)
  );
}

function cacheTitle(href: string): Promise<string | null> {
  const cached = titleCache.get(href);
  if (cached instanceof Promise) return cached;
  if (cached !== undefined) return Promise.resolve(cached);

  const promise = fetchLinkPreviewTitle(href)
    .then((title) => {
      titleCache.set(href, title);
      return title;
    })
    .catch(() => {
      titleCache.set(href, null);
      return null;
    });
  titleCache.set(href, promise);
  return promise;
}

export function useResolvedLinkPreviews(
  previews: SupportedLinkPreview[],
): SupportedLinkPreview[] {
  const [resolvedTitles, setResolvedTitles] = React.useState<
    Record<string, string>
  >({});

  React.useEffect(() => {
    let cancelled = false;
    const pending = previews.filter(shouldResolveTitle);
    if (pending.length === 0) return undefined;

    for (const preview of pending) {
      const cached = titleCache.get(preview.href);
      if (typeof cached === "string" && cached) {
        setResolvedTitles((current) =>
          current[preview.href] === cached
            ? current
            : { ...current, [preview.href]: cached },
        );
        continue;
      }

      void cacheTitle(preview.href).then((title) => {
        if (cancelled || !title) return;
        setResolvedTitles((current) =>
          current[preview.href] === title
            ? current
            : { ...current, [preview.href]: title },
        );
      });
    }

    return () => {
      cancelled = true;
    };
  }, [previews]);

  return React.useMemo(
    () =>
      previews.map((preview) => {
        const title = resolvedTitles[preview.href];
        return title ? { ...preview, title } : preview;
      }),
    [previews, resolvedTitles],
  );
}
