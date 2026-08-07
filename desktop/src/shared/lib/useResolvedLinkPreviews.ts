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
 * Generation counter incremented on every `resetLinkPreviewTitleCache` call.
 * Each in-flight promise captures the generation at creation time and only
 * writes back if no reset has happened since — preventing a resolved promise
 * from a previous community from repopulating stale entries into the fresh
 * cache of a new community.
 */
let cacheGeneration = 0;

/**
 * Buzz entity titles come from relay events, so they are community-scoped —
 * wired into `resetCommunityState()` (see useCommunityInit.ts) to avoid
 * leaking titles across community switches.
 */
export function resetLinkPreviewTitleCache(): void {
  cacheGeneration += 1;
  titleCache.clear();
}

/**
 * Returns the current cache generation counter. Used in tests to verify that
 * `resetLinkPreviewTitleCache` increments the generation so stale in-flight
 * promises cannot seed the new cache.
 *
 * @internal test-only export
 */
export function getLinkPreviewCacheGeneration(): number {
  return cacheGeneration;
}

function fetchLinkPreviewTitle(href: string): Promise<string | null> {
  return invokeTauri<string | null>("fetch_link_preview_title", { href });
}

/**
 * Resolve a Buzz PR/issue card title from the relay event's `subject` tag
 * (first content line as fallback — the same precedence the projects views
 * use).
 *
 * Security: the fetched event's canonical `a` tag must equal
 * `30617:<owner>:<d>` from the link before we adopt its title. Without this
 * check, a crafted link could pair the real title of a legitimate PR with an
 * unrelated repository destination.
 */
async function fetchBuzzEntityTitle(href: string): Promise<string | null> {
  const parsed = parseEntityLink(href);
  if (!parsed.ok || parsed.value.type === "repo") return null;

  const { id, owner, dtag } = parsed.value;
  const expectedCoordinate = `30617:${owner}:${dtag}`;

  const events = await relayClient.fetchEvents({
    kinds: [
      parsed.value.type === "pr" ? KIND_GIT_PULL_REQUEST : KIND_GIT_ISSUE,
    ],
    ids: [id],
    limit: 1,
  });
  const event = events[0];
  if (!event) return null;

  // Verify the event belongs to the claimed repository coordinate.
  const aTag = event.tags.find(
    (tag) => tag[0] === "a" && tag[1] === expectedCoordinate,
  );
  if (!aTag) return null;

  const subject = event.tags.find((tag) => tag[0] === "subject")?.[1];
  return subject || event.content.split("\n")[0] || null;
}

/**
 * Returns true when the preview's current title is still the auto-generated
 * fallback and a relay lookup should be attempted to replace it. Returns false
 * once the user has applied a markdown label (`[My label](link)`) so that the
 * label wins over any cached relay title.
 *
 * Exported for unit testing of the label-must-win invariant.
 */
export function shouldResolveTitle(preview: SupportedLinkPreview): boolean {
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

  const generation = cacheGeneration;
  const promise = resolveTitle(preview)
    .then((title) => {
      // Only write back if no community switch has happened since we started.
      if (cacheGeneration === generation) {
        titleCache.set(preview.href, title);
      }
      return title;
    })
    .catch(() => {
      if (cacheGeneration === generation) {
        titleCache.set(preview.href, null);
      }
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
        // Only apply a relay-resolved title while the preview still has the
        // fallback title (i.e. no markdown-label override). If the user edits
        // a bare link into `[My label](same-link)`, `shouldResolveTitle`
        // returns false and the label wins — the cached relay title is not
        // applied to prevent it from silently overriding the explicit label.
        return title && shouldResolveTitle(preview)
          ? { ...preview, title }
          : preview;
      }),
    [previews, resolvedTitles],
  );
}
