import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";

/**
 * Module-level cache so each URL is fetched once per app session no matter
 * how many messages mention it (same pattern as `useResolvedLinkPreviews`).
 */
const titleCache = new Map<string, Promise<string | null> | string | null>();

function cacheTitle(href: string): Promise<string | null> {
  const cached = titleCache.get(href);
  if (cached instanceof Promise) return cached;
  if (cached !== undefined) return Promise.resolve(cached);

  const promise = invokeTauri<string | null>("fetch_page_title", { href })
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

/**
 * Best-effort page title for a URL, or null while loading / when the page
 * can't be fetched (auth walls, non-HTML, timeouts).
 */
export function usePageTitle(href: string): string | null {
  const cached = titleCache.get(href);
  const [title, setTitle] = React.useState<string | null>(
    typeof cached === "string" ? cached : null,
  );

  React.useEffect(() => {
    let cancelled = false;
    void cacheTitle(href).then((resolved) => {
      if (!cancelled && resolved) setTitle(resolved);
    });
    return () => {
      cancelled = true;
    };
  }, [href]);

  return title;
}
