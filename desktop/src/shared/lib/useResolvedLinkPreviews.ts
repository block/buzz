import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PULL_REQUEST,
} from "@/shared/constants/kinds";

import { parseEntityLink } from "./entityLink";
import {
  buzzEntityFallbackTitle,
  type SupportedLinkPreview,
} from "./linkPreview";

const GOOGLE_FALLBACK_TITLES = new Set([
  "Drive file",
  "Drive folder",
  "Document",
  "Spreadsheet",
  "Presentation",
]);

const titleCache = new Map<string, Promise<string | null> | string | null>();

/**
 * Buzz entity titles come from relay events, so they are community-scoped —
 * wired into `resetCommunityState()` (see useCommunityInit.ts) to avoid
 * leaking titles across community switches.
 */
export function resetLinkPreviewTitleCache(): void {
  titleCache.clear();
}

function fetchLinkPreviewTitle(href: string): Promise<string | null> {
  return invokeTauri<string | null>("fetch_link_preview_title", { href });
}

/**
 * Resolve a Buzz PR/issue card title from the relay event's `subject` tag
 * (first content line as fallback — the same precedence the projects views
 * use).
 */
async function fetchBuzzEntityTitle(href: string): Promise<string | null> {
  const parsed = parseEntityLink(href);
  if (!parsed.ok || parsed.value.type === "repo") return null;

  const events = await relayClient.fetchEvents({
    kinds: [
      parsed.value.type === "pr" ? KIND_GIT_PULL_REQUEST : KIND_GIT_ISSUE,
    ],
    ids: [parsed.value.id],
    limit: 1,
  });
  const event = events[0];
  if (!event) return null;

  const subject = event.tags.find((tag) => tag[0] === "subject")?.[1];
  return subject || event.content.split("\n")[0] || null;
}

function shouldResolveTitle(preview: SupportedLinkPreview): boolean {
  if (preview.kind === "buzz-pull-request" || preview.kind === "buzz-issue") {
    // A markdown-label override replaces the fallback title and must win
    // over the relay lookup.
    const parsed = parseEntityLink(preview.href);
    return parsed.ok && preview.title === buzzEntityFallbackTitle(parsed.value);
  }

  return (
    preview.kind.startsWith("google-") &&
    GOOGLE_FALLBACK_TITLES.has(preview.title)
  );
}

function resolveTitle(preview: SupportedLinkPreview): Promise<string | null> {
  return preview.href.startsWith("buzz://")
    ? fetchBuzzEntityTitle(preview.href)
    : fetchLinkPreviewTitle(preview.href);
}

function cacheTitle(preview: SupportedLinkPreview): Promise<string | null> {
  const cached = titleCache.get(preview.href);
  if (cached instanceof Promise) return cached;
  if (cached !== undefined) return Promise.resolve(cached);

  const promise = resolveTitle(preview)
    .then((title) => {
      titleCache.set(preview.href, title);
      return title;
    })
    .catch(() => {
      titleCache.set(preview.href, null);
      return null;
    });
  titleCache.set(preview.href, promise);
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

      void cacheTitle(preview).then((title) => {
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
