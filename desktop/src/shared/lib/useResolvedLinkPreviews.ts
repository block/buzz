import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  eventToExplicitProject,
  eventToRepository,
} from "@/features/projects/projectModels";
import { eventToProjectIssue } from "@/features/projects/projectIssues.mjs";
import { eventToProjectPullRequest } from "@/features/projects/projectPullRequests.mjs";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PULL_REQUEST,
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";

import { isEntityLink, parseEntityLink } from "./entityLink";
import {
  buzzEntityFallbackTitle,
  type SupportedLinkPreview,
} from "./linkPreview";

type LinkPreviewImageFetchState =
  | "none"
  | "image"
  | "transient_failure"
  | "rejected";

export type LinkPreviewMetadata = {
  title: string;
  siteName: string | null;
  description: string | null;
  imageDataUrl: string | null;
  imageDomain: string | null;
  imageFetchState?: LinkPreviewImageFetchState;
  imageRetryAfterMs?: number | null;
  faviconDataUrl?: string | null;
};

type MetadataCacheEntry = {
  expiresAt: number | null;
  metadata: LinkPreviewMetadata | null;
};

type MetadataLoadResult = MetadataCacheEntry & {
  key: string;
};

const DEFAULT_TRANSIENT_RETRY_MS = 30_000;
const NULL_METADATA_RETRY_MS = 5 * 60_000;
const MAX_CONCURRENT_METADATA_FETCHES = 2;
// Keep a just-removed request cacheable through a brief edit/re-entry gap, but
// never let unobserved native work linger indefinitely. New queued demand
// reclaims these loads immediately rather than waiting for this grace period.
const ORPHANED_METADATA_LOAD_GRACE_MS = 1_000;

/**
 * React may flush an interaction-triggered effect before the browser paints.
 * Start uncached preview I/O after a frame plus a task boundary so the pasted
 * text and loading card are visible before native IPC work begins.
 */
function scheduleAfterPaint(task: () => void): () => void {
  let frameId: number | null = null;
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  const run = () => {
    timeoutId = setTimeout(task, 0);
  };

  if (typeof requestAnimationFrame === "function") {
    frameId = requestAnimationFrame(run);
  } else {
    run();
  }

  return () => {
    if (frameId !== null) cancelAnimationFrame(frameId);
    if (timeoutId !== null) clearTimeout(timeoutId);
  };
}

function metadataCacheKey(href: string): string {
  try {
    const url = new URL(href);
    url.hash = "";
    return url.href;
  } catch {
    return href.split("#", 1)[0] ?? href;
  }
}

function isNegativeMetadata(metadata: LinkPreviewMetadata | null): boolean {
  // A cached NEGATIVE is a result that should be retried when the URL freshly
  // re-enters the composer: a hard miss (null) or a transient image failure.
  // A healthy hit (`image`/`rejected`/no state) is a settled positive.
  return metadata === null || metadata.imageFetchState === "transient_failure";
}

function metadataExpiry(
  metadata: LinkPreviewMetadata | null,
  now: number,
): number | null {
  if (metadata === null) return now + NULL_METADATA_RETRY_MS;
  if (metadata.imageFetchState !== "transient_failure") return null;
  const retryAfterMs =
    typeof metadata.imageRetryAfterMs === "number" &&
    Number.isFinite(metadata.imageRetryAfterMs)
      ? Math.max(1_000, metadata.imageRetryAfterMs)
      : DEFAULT_TRANSIENT_RETRY_MS;
  return now + retryAfterMs;
}

type PendingMetadataLoad = {
  controller: AbortController;
  consumers: number;
  orphanTimer: ReturnType<typeof setTimeout> | null;
  promise: Promise<MetadataLoadResult>;
};

type MetadataCacheValue = MetadataCacheEntry | PendingMetadataLoad;

function isPendingMetadataLoad(
  value: MetadataCacheValue,
): value is PendingMetadataLoad {
  return "promise" in value;
}

function abortError(): DOMException {
  return new DOMException("Link preview fetch cancelled", "AbortError");
}

function createTaskScheduler(concurrency: number) {
  const pending: Array<{
    reject: (reason?: unknown) => void;
    run: () => void;
    signal: AbortSignal;
  }> = [];
  let active = 0;

  const drain = () => {
    while (active < concurrency) {
      const next = pending.shift();
      if (!next) return;
      if (next.signal.aborted) {
        next.reject(abortError());
        continue;
      }
      active += 1;
      next.run();
    }
  };

  return <T>(task: () => Promise<T>, signal: AbortSignal): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      const entry = {
        reject,
        signal,
        run: () => {
          signal.removeEventListener("abort", cancelPending);
          void task()
            .then(resolve, reject)
            .finally(() => {
              active -= 1;
              drain();
            });
        },
      };
      const cancelPending = () => {
        const index = pending.indexOf(entry);
        if (index < 0) return;
        pending.splice(index, 1);
        reject(abortError());
        drain();
      };
      signal.addEventListener("abort", cancelPending, { once: true });
      pending.push(entry);
      drain();
    });
}

function createMetadataLoader({
  concurrency = MAX_CONCURRENT_METADATA_FETCHES,
  fetcher,
  now = Date.now,
}: {
  concurrency?: number;
  fetcher: (
    href: string,
    signal: AbortSignal,
  ) => Promise<LinkPreviewMetadata | null>;
  now?: () => number;
}) {
  const cache = new Map<string, MetadataCacheValue>();
  const schedule = createTaskScheduler(Math.max(1, concurrency));
  let generation = 0;

  const peek = (href: string): MetadataLoadResult | undefined => {
    const key = metadataCacheKey(href);
    const cached = cache.get(key);
    if (!cached || isPendingMetadataLoad(cached)) return undefined;
    if (cached.expiresAt !== null && cached.expiresAt <= now()) {
      cache.delete(key);
      return undefined;
    }
    return { key, ...cached };
  };

  const abortPending = (key: string, pending: PendingMetadataLoad) => {
    if (pending.orphanTimer !== null) clearTimeout(pending.orphanTimer);
    pending.orphanTimer = null;
    if (cache.get(key) === pending) cache.delete(key);
    pending.controller.abort();
  };

  const release = (key: string, pending: PendingMetadataLoad) => {
    if (pending.consumers <= 0) return;
    pending.consumers -= 1;
    if (
      pending.consumers > 0 ||
      pending.orphanTimer !== null ||
      cache.get(key) !== pending
    ) {
      return;
    }
    pending.orphanTimer = setTimeout(
      () => abortPending(key, pending),
      ORPHANED_METADATA_LOAD_GRACE_MS,
    );
  };

  const createRelease = (key: string, pending: PendingMetadataLoad) => {
    let released = false;
    return () => {
      if (released) return;
      released = true;
      release(key, pending);
    };
  };

  const abortOrphanedLoads = (exceptKey: string) => {
    for (const [key, cached] of cache) {
      if (
        key !== exceptKey &&
        isPendingMetadataLoad(cached) &&
        cached.consumers === 0
      ) {
        abortPending(key, cached);
      }
    }
  };

  const load = (
    href: string,
  ): { cancel: () => void; promise: Promise<MetadataLoadResult> } => {
    const key = metadataCacheKey(href);
    const cached = cache.get(key);
    if (cached && isPendingMetadataLoad(cached)) {
      if (cached.orphanTimer !== null) clearTimeout(cached.orphanTimer);
      cached.orphanTimer = null;
      cached.consumers += 1;
      return {
        cancel: createRelease(key, cached),
        promise: cached.promise,
      };
    }
    if (cached) {
      if (cached.expiresAt === null || cached.expiresAt > now()) {
        return {
          cancel: () => undefined,
          promise: Promise.resolve({ key, ...cached }),
        };
      }
      cache.delete(key);
    }

    abortOrphanedLoads(key);
    const requestGeneration = generation;
    const controller = new AbortController();
    const pending: PendingMetadataLoad = {
      controller,
      consumers: 1,
      orphanTimer: null,
      promise: Promise.resolve({
        expiresAt: null,
        key,
        metadata: null,
      }),
    };
    pending.promise = schedule(
      () => fetcher(href, controller.signal),
      controller.signal,
    )
      .catch((error) => {
        if (controller.signal.aborted) throw error;
        return null;
      })
      .then((metadata) => {
        if (pending.orphanTimer !== null) clearTimeout(pending.orphanTimer);
        pending.orphanTimer = null;
        const entry = {
          expiresAt: metadataExpiry(metadata, now()),
          metadata,
        };
        if (
          requestGeneration === generation &&
          cache.get(key) === pending &&
          !controller.signal.aborted
        ) {
          cache.set(key, entry);
        }
        return { key, ...entry };
      });
    cache.set(key, pending);
    return {
      cancel: createRelease(key, pending),
      promise: pending.promise,
    };
  };

  return {
    deleteKey(key: string) {
      cache.delete(key);
    },
    /**
     * Drop a cached NEGATIVE result (a resolved null or a transient failure) so
     * the next load refetches. Used when a URL freshly enters the composer: a
     * user pasting a link that previously blanked should get a new attempt now,
     * not the stale miss. A healthy cached hit and an in-flight fetch are left
     * untouched, so passive scroll re-renders still ride the cache as before.
     * Returns whether a negative entry was actually dropped, so callers can
     * invalidate their own derived state (e.g. retained React metadata) in step.
     */
    invalidateNegative(href: string): boolean {
      const key = metadataCacheKey(href);
      const cached = cache.get(key);
      if (!cached || isPendingMetadataLoad(cached)) return false;
      if (!isNegativeMetadata(cached.metadata)) return false;
      cache.delete(key);
      return true;
    },
    load,
    peek,
    reset() {
      generation += 1;
      for (const [key, cached] of cache) {
        if (isPendingMetadataLoad(cached)) abortPending(key, cached);
      }
      cache.clear();
    },
  };
}

function fetchLinkPreviewMetadata(
  href: string,
  signal: AbortSignal,
): Promise<LinkPreviewMetadata | null> {
  const requestId = crypto.randomUUID();
  const startedAt = performance.now();
  const logResult = (
    result: LinkPreviewMetadata | null,
  ): LinkPreviewMetadata | null => {
    if (import.meta.env?.DEV) {
      console.info("[link-preview] metadata fetch completed", {
        href,
        elapsedMs: Math.round(performance.now() - startedAt),
        result: result === null ? "miss" : "hit",
        imageFetchState: result?.imageFetchState ?? "none",
        imageRetryAfterMs: result?.imageRetryAfterMs ?? null,
        hasImage: Boolean(result?.imageDataUrl && result.imageDomain),
        hasFavicon: Boolean(result?.faviconDataUrl),
      });
    }
    return result;
  };
  const logFailure = (error: unknown): never => {
    if (import.meta.env?.DEV) {
      console.warn("[link-preview] metadata fetch failed", {
        href,
        elapsedMs: Math.round(performance.now() - startedAt),
        error,
      });
    }
    throw error;
  };

  const request = invokeTauri<LinkPreviewMetadata | null>(
    "fetch_link_preview_metadata",
    {
      href,
      requestId,
    },
  );
  const onAbort = () => {
    void invokeTauri("cancel_link_preview_metadata", { requestId }).catch(
      () => undefined,
    );
  };
  signal.addEventListener("abort", onAbort, { once: true });
  if (signal.aborted) onAbort();
  return request.then(logResult, logFailure).finally(() => {
    signal.removeEventListener("abort", onAbort);
    void invokeTauri("release_link_preview_metadata", { requestId }).catch(
      () => undefined,
    );
  });
}

const metadataLoader = createMetadataLoader({
  fetcher: fetchLinkPreviewMetadata,
});

/** Share the same deduplicated metadata job between composer rendering and send preparation. */
export function loadLinkPreviewMetadata(href: string): {
  cancel: () => void;
  promise: Promise<LinkPreviewMetadata | null>;
} {
  const load = metadataLoader.load(href);
  return {
    cancel: load.cancel,
    promise: load.promise.then((result) => result.metadata),
  };
}
type EntityEventFetcher = (
  filter: Parameters<typeof relayClient.fetchEvents>[0],
) => Promise<RelayEvent[]>;

function compactMetadata(
  parts: Array<string | null | undefined>,
): string | null {
  const values = parts.filter((part): part is string => Boolean(part));
  return values.length > 0 ? values.join(" · ") : null;
}

/** Resolve builder-focused metadata only from the active relay. */
export async function fetchBuzzEntityMetadata(
  href: string,
  fetchEvents: EntityEventFetcher = (filter) => relayClient.fetchEvents(filter),
): Promise<LinkPreviewMetadata | null> {
  const parsed = parseEntityLink(href);
  if (!parsed.ok) return null;

  const { owner, dtag } = parsed.value;
  if (parsed.value.type === "project") {
    const projectAddress = `${KIND_PROJECT_ANNOUNCEMENT}:${owner}:${dtag}`;
    const projectEvents = await fetchEvents({
      kinds: [KIND_PROJECT_ANNOUNCEMENT],
      authors: [owner],
      "#d": [dtag],
      limit: 1,
    });
    const project = projectEvents
      .map((event) => eventToExplicitProject(event, new Map(), new Map()))
      .find((candidate) => candidate?.projectAddress === projectAddress);
    if (!project) return null;

    const repositoryCount = project.repositoryAddresses.length;
    return {
      siteName: project.name,
      faviconDataUrl: null,
      imageDataUrl: null,
      imageDomain: null,
      title: project.description || project.name,
      description: compactMetadata([
        repositoryCount > 0
          ? `${repositoryCount} ${repositoryCount === 1 ? "repository" : "repositories"}`
          : null,
      ]),
    };
  }

  const repoAddress = `${KIND_REPO_ANNOUNCEMENT}:${owner}:${dtag}`;
  const repoEvents = await fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    authors: [owner],
    "#d": [dtag],
    limit: 1,
  });
  const repository = repoEvents
    .map((event) => eventToRepository(event))
    .find((candidate) => candidate?.repoAddress === repoAddress);
  if (!repository) return null;

  const base = {
    siteName: repository.name,
    faviconDataUrl: null,
    imageDataUrl: null,
    imageDomain: null,
  };
  if (parsed.value.type === "repo") {
    return {
      ...base,
      title: repository.description || repository.name,
      description: compactMetadata([
        repository.status,
        `default: ${repository.defaultBranch}`,
      ]),
    };
  }

  const { id, type } = parsed.value;
  const rootEvents = await fetchEvents({
    kinds: [type === "pr" ? KIND_GIT_PULL_REQUEST : KIND_GIT_ISSUE],
    ids: [id],
    limit: 1,
  });
  const root = rootEvents.find((event) => {
    const repositoryTags = event.tags.filter((tag) => tag[0] === "a");
    return (
      event.id === id &&
      repositoryTags.length === 1 &&
      repositoryTags[0][1] === repoAddress
    );
  });
  if (!root) return null;

  const title =
    type === "issue"
      ? eventToProjectIssue(root).title
      : eventToProjectPullRequest(root).title;
  return {
    ...base,
    title,
    // Issue and PR bodies are markdown and often begin with headings. Keep
    // their fetched subject as tooltip/card context without fetching or
    // flattening body/status metadata into an unrendered description string.
    description: null,
  };
}

const entityMetadataLoader = createMetadataLoader({
  fetcher: (href) => fetchBuzzEntityMetadata(href),
});

/** Share deduplicated relay-native entity metadata across cards and inline tooltips. */
export async function loadBuzzEntityMetadata(
  href: string,
): Promise<LinkPreviewMetadata | null> {
  const load = entityMetadataLoader.load(href);
  try {
    return (await load.promise).metadata;
  } finally {
    load.cancel();
  }
}

/** Clear ephemeral metadata when the active relay/community changes. */
export function resetLinkPreviewMetadataCache(): void {
  metadataLoader.reset();
  entityMetadataLoader.reset();
}

export type LinkPreviewImageState = "pending" | "image" | "fallback" | "none";

export type ResolvedLinkPreview = SupportedLinkPreview & {
  description?: string | null;
  faviconDataUrl?: string | null;
  imageState: LinkPreviewImageState;
  /** Metadata extraction completed successfully; safe to snapshot after media uploads. */
  snapshotReady?: boolean;
};

type ResolvedMetadataByHref = Record<
  string,
  LinkPreviewMetadata | null | undefined
>;

/** Only auto-generated titles may be replaced; explicit markdown labels win. */
export function shouldResolveTitle(preview: SupportedLinkPreview): boolean {
  if (!isEntityLink(preview.href)) return true;
  const parsed = parseEntityLink(preview.href);
  return parsed.ok && preview.title === buzzEntityFallbackTitle(parsed.value);
}

export function resolveLinkPreview(
  preview: SupportedLinkPreview,
  metadata: LinkPreviewMetadata | null | undefined,
): ResolvedLinkPreview {
  if (metadata === undefined) {
    return {
      ...preview,
      imageState: isBuzzEntityPreview(preview) ? "none" : "pending",
    };
  }
  if (metadata === null) {
    return { ...preview, imageState: "none" };
  }

  const hasImage = Boolean(metadata.imageDataUrl && metadata.imageDomain);
  const imageState: LinkPreviewImageState = hasImage
    ? "image"
    : metadata.imageFetchState === "image" ||
        metadata.imageFetchState === "transient_failure" ||
        metadata.imageFetchState === "rejected"
      ? "fallback"
      : "none";
  return {
    ...preview,
    snapshotReady: !preview.href.startsWith("buzz://"),
    title: shouldResolveTitle(preview) ? metadata.title : preview.title,
    description: metadata.description,
    faviconDataUrl: metadata.faviconDataUrl,
    provider:
      (preview.kind === "generic-link" || isBuzzEntityPreview(preview)) &&
      metadata.siteName
        ? metadata.siteName
        : preview.provider,
    imageDataUrl: hasImage ? metadata.imageDataUrl : null,
    imageDomain: hasImage ? metadata.imageDomain : null,
    imageState,
  };
}

export function isBuzzEntityPreview(preview: SupportedLinkPreview): boolean {
  return (
    preview.kind === "buzz-pull-request" ||
    preview.kind === "buzz-issue" ||
    preview.kind === "buzz-repository" ||
    preview.kind === "buzz-project"
  );
}

/**
 * Recipient-side `buzz://` entity cards must render even when the relay
 * lookup yields no metadata: `useResolvedLinkPreviews` drops null-metadata
 * previews (correct for external links — no metadata means no card), but
 * entity links always carry a usable fallback title (the repo d-tag, or
 * `<dtag> #<id8>` for PRs/issues — see `buzzEntityFallbackTitle`). Re-adds
 * recognized entity previews on their fallback title; non-entity previews
 * keep the hook's drop behavior.
 */
export function withEntityFallbacks(
  previews: SupportedLinkPreview[],
  resolved: ResolvedLinkPreview[],
): ResolvedLinkPreview[] {
  const byHref = new Map(resolved.map((preview) => [preview.href, preview]));
  return previews.flatMap((preview) => {
    const match = byHref.get(preview.href);
    if (match) return [match];
    return isBuzzEntityPreview(preview)
      ? [{ ...preview, imageState: "none" as const }]
      : [];
  });
}

export function useResolvedLinkPreviews(
  previews: SupportedLinkPreview[],
  {
    refetchNewNegatives = false,
    liveHrefs,
    liveHrefVersions,
  }: {
    /**
     * When a preview href is newly present since the last run, drop any cached
     * NEGATIVE (null/transient-fail) metadata for it so it refetches instead of
     * resolving to a stale miss. Used by the composer: a freshly pasted link
     * should get a new attempt. Off by default so passive renders (the message
     * list) keep riding the cache. Healthy cached hits are never invalidated.
     */
    refetchNewNegatives?: boolean;
    /**
     * The hrefs present in the caller's LIVE (undebounced) content. When given,
     * newness is judged against this set instead of the resolved `previews`, so
     * a URL that leaves and re-enters the live content is treated as re-entered
     * even when a debounce swallowed the intermediate empty state (the composer
     * debounces resolution, so `previews` may never observe the URL leaving).
     * Resolution timing still follows `previews`; only the invalidation decision
     * uses this. Omit to track newness against `previews` (the default).
     */
    liveHrefs?: readonly string[];
    /**
     * Per-href entry versions captured at the live editor-update boundary.
     * Unlike committed href-set equality, a bumped version preserves an
     * intermediate leave/re-entry even when React batches both updates into one
     * commit with the same final href set.
     */
    liveHrefVersions?: ReadonlyMap<string, number>;
  } = {},
): ResolvedLinkPreview[] {
  const [resolvedMetadata, setResolvedMetadata] =
    React.useState<ResolvedMetadataByHref>({});
  const [retryGeneration, setRetryGeneration] = React.useState(0);
  const seenHrefsRef = React.useRef<Set<string>>(new Set());
  const handledHrefVersionsRef = React.useRef<Map<string, number>>(new Map());
  // Drive newness tracking from a stable string key so an equivalent href list
  // does not restart the effect. The effect closes over the committed render's
  // hrefs; do not mirror them into a ref during render, because an abandoned
  // concurrent render could otherwise leak uncommitted presence into the live
  // effect from the previous commit.
  const currentHrefs = liveHrefs ?? previews.map((preview) => preview.href);
  const newnessKey = currentHrefs
    .map((href) => `${href}\0${liveHrefVersions?.get(href) ?? ""}`)
    .join("\n");
  // biome-ignore lint/correctness/useExhaustiveDependencies: newnessKey is the stable identity for currentHrefs; depending on the freshly allocated array would rerun this effect every render.
  React.useEffect(() => {
    let cancelled = false;
    let retryAt = Number.POSITIVE_INFINITY;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

    if (refetchNewNegatives) {
      // Invalidate first, before the peek/load loop below reads the cache, so a
      // newly-present href loads fresh instead of resolving to its stale miss.
      // buzz:// entity links resolve off the relay, not this cache — skip them.
      // Newness is judged against the live href set when supplied (so a
      // debounce-swallowed leave/re-entry still counts), else against previews.
      const seen = seenHrefsRef.current;
      const handledVersions = handledHrefVersionsRef.current;
      const liveNow = currentHrefs;
      // Preserve handled hrefs only while they remain live. A live href is not
      // marked handled until its debounced preview exists and invalidation has
      // actually been attempted; otherwise blank -> paste would consume
      // newness during the 350ms debounce and later reuse the stale negative.
      const next = new Set<string>(
        [...seen].filter((href) => liveNow.includes(href)),
      );
      const nextVersions = new Map(
        [...handledVersions].filter(([href]) => liveNow.includes(href)),
      );
      const reenteredKeys: string[] = [];
      for (const preview of previews) {
        const version = liveHrefVersions?.get(preview.href);
        const alreadyHandled =
          version === undefined
            ? seen.has(preview.href)
            : handledVersions.get(preview.href) === version;
        if (
          alreadyHandled ||
          !liveNow.includes(preview.href) ||
          preview.href.startsWith("buzz://")
        ) {
          continue;
        }
        // Drop any settled NEGATIVE from the SHARED loader cache so the load
        // below refetches instead of resolving to the stale miss. (A no-op when
        // the shared entry is healthy, in-flight, or absent.)
        metadataLoader.invalidateNegative(preview.href);
        reenteredKeys.push(metadataCacheKey(preview.href));
        next.add(preview.href);
        if (version !== undefined) nextVersions.set(preview.href, version);
      }
      seenHrefsRef.current = next;
      handledHrefVersionsRef.current = nextVersions;
      // Dropping the loader entry alone is not enough: this hook retains its own
      // resolved metadata, and the render that scheduled this effect already
      // read the stale negative from it. Clear this hook's OWN negative key for
      // every re-entered href — gating on whether the shared loader dropped a
      // settled entry misses the case where another hook left an in-flight
      // Promise in the shared cache (invalidateNegative leaves Promises alone
      // and the loop below merely coalesces onto it), which would otherwise
      // keep this hook's retained `transient_failure` as a `snapshotReady`
      // fallback the composer could turn into a sendable snapshot tag from stale
      // metadata until that fetch resolves. Clearing the local negative renders
      // the re-entered link as pending until the fresh load wins. Healthy local
      // hits are kept, so passive re-renders still show their card instantly.
      if (reenteredKeys.length > 0) {
        setResolvedMetadata((current) => {
          let changed = false;
          const nextMetadata = { ...current };
          for (const key of reenteredKeys) {
            const value = nextMetadata[key];
            if (value !== undefined && isNegativeMetadata(value)) {
              delete nextMetadata[key];
              changed = true;
            }
          }
          return changed ? nextMetadata : current;
        });
      }
    }

    const scheduleRetry = (
      { expiresAt, key }: Pick<MetadataLoadResult, "expiresAt" | "key">,
      loader: typeof metadataLoader,
    ) => {
      if (expiresAt === null || expiresAt >= retryAt) return;
      retryAt = expiresAt;
      if (retryTimer !== null) clearTimeout(retryTimer);
      retryTimer = setTimeout(
        () => {
          loader.deleteKey(key);
          setResolvedMetadata((current) => {
            if (!(key in current)) return current;
            const next = { ...current };
            delete next[key];
            return next;
          });
          setRetryGeneration(retryGeneration + 1);
        },
        Math.max(0, expiresAt - Date.now()),
      );
    };

    const cancelScheduledLoads: Array<() => void> = [];
    for (const preview of previews) {
      const loader = preview.href.startsWith("buzz://")
        ? entityMetadataLoader
        : metadataLoader;
      const cached = loader.peek(preview.href);
      if (cached !== undefined) {
        setResolvedMetadata((current) =>
          current[cached.key] === cached.metadata
            ? current
            : { ...current, [cached.key]: cached.metadata },
        );
        scheduleRetry(cached, loader);
        continue;
      }

      cancelScheduledLoads.push(
        scheduleAfterPaint(() => {
          const load = loader.load(preview.href);
          cancelScheduledLoads.push(load.cancel);
          void load.promise
            .then((result) => {
              if (cancelled) return;
              setResolvedMetadata((current) =>
                current[result.key] === result.metadata
                  ? current
                  : { ...current, [result.key]: result.metadata },
              );
              scheduleRetry(result, loader);
            })
            .catch(() => undefined);
        }),
      );
    }

    return () => {
      cancelled = true;
      for (const cancel of cancelScheduledLoads) cancel();
      if (retryTimer !== null) clearTimeout(retryTimer);
    };
  }, [previews, refetchNewNegatives, retryGeneration, newnessKey]);

  return React.useMemo(
    () =>
      previews.flatMap((preview) => {
        const metadata = resolvedMetadata[metadataCacheKey(preview.href)];
        return metadata === null ? [] : [resolveLinkPreview(preview, metadata)];
      }),
    [previews, resolvedMetadata],
  );
}

export const __linkPreviewMetadataTest = {
  createMetadataLoader,
  createTaskScheduler,
  metadataCacheKey,
  metadataExpiry,
};
