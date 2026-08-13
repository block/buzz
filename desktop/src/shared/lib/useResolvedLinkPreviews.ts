import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { eventToRepository } from "@/features/projects/projectModels";
import { eventToProjectIssue } from "@/features/projects/projectIssues.mjs";
import { eventToProjectPullRequest } from "@/features/projects/projectPullRequests.mjs";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";

import { parseEntityLink } from "./entityLink";
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
/** Floor for any transient retry delay so a `Retry-After: 0` (or a stale
 * HTTP-date already in the past) cannot collapse `expiresAt` onto `now` and
 * spin the expiry timer into a tight refetch loop while the failure persists. */
const MIN_TRANSIENT_RETRY_MS = 1_000;
const MAX_CONCURRENT_METADATA_FETCHES = 2;
/** Backoff before the single inline retry that a transient blip earns before
 * it falls into the longer {@link DEFAULT_TRANSIENT_RETRY_MS} self-heal wait. */
const INLINE_RETRY_BACKOFF_MS = 750;
/** Prefix the native fetcher uses to mark a 429. A rate limit must NOT get the
 * fast inline retry (we would just be throttled again); it waits instead. Kept
 * in sync with RATE_LIMITED_ERROR_PREFIX in link_preview.rs. */
const RATE_LIMITED_ERROR_PREFIX = "link preview rate limited";
/** Prefix the native fetcher uses to mark a permanent validation/policy
 * rejection (non-HTTPS/credentialed URL, non-default port, private/reserved
 * host, unparseable URL or redirect). These must NOT retry or self-heal poll —
 * the work would only be rejected again — so they cache as a hard miss. Kept in
 * sync with PERMANENTLY_REJECTED_ERROR_PREFIX in link_preview.rs. */
const PERMANENTLY_REJECTED_ERROR_PREFIX = "link preview rejected";

type FetchFailure =
  /** A permanent validation/policy rejection: cache as a hard miss, no retry. */
  | { kind: "rejected" }
  /** A 429 rate limit: skip the inline retry and wait out its Retry-After. */
  | { kind: "rate_limited"; retryAfterMs?: number }
  /** An ordinary transient blip (timeout/5xx/network): earns one inline retry. */
  | { kind: "transient" };

/** Classify a rejected native fetch. A genuine "no metadata" result resolves
 * (it does not reject), so every rejection is a failure of one of three kinds:
 * a permanent validation/policy rejection (never retry), a 429 rate limit (wait
 * out its Retry-After, no inline retry), or an ordinary transient blip. */
function classifyFetchFailure(reason: unknown): FetchFailure {
  const message = reason instanceof Error ? reason.message : String(reason);
  if (message.startsWith(PERMANENTLY_REJECTED_ERROR_PREFIX)) {
    return { kind: "rejected" };
  }
  if (!message.startsWith(RATE_LIMITED_ERROR_PREFIX)) {
    return { kind: "transient" };
  }
  const match = /retry-after (\d+)/.exec(message);
  const retryAfterSeconds = match ? Number.parseInt(match[1], 10) : NaN;
  return {
    kind: "rate_limited",
    retryAfterMs: Number.isFinite(retryAfterSeconds)
      ? retryAfterSeconds * 1_000
      : undefined,
  };
}

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

/** Outcome of a coalesced fetch attempt sequence: a successful (or genuinely
 * empty) result, an ordinary transient failure that self-heals soon, or a
 * permanent rejection cached as a hard miss. */
type AttemptOutcome = "ok" | "transient" | "rejected";

type AttemptResult = {
  metadata: LinkPreviewMetadata | null;
  outcome: AttemptOutcome;
  /** Delay before the self-heal retry when {@link outcome} is "transient". */
  transientRetryMs: number;
};

function metadataExpiry(
  metadata: LinkPreviewMetadata | null,
  now: number,
  outcome: AttemptOutcome = "ok",
  transientRetryMs = DEFAULT_TRANSIENT_RETRY_MS,
): number | null {
  // A transient failure (timeout/429/5xx/network) must not stick for the full
  // miss TTL: one unlucky fetch would otherwise blank the card for 5 minutes
  // even though the link is perfectly valid. Retry it soon instead — after the
  // server's Retry-After when it gave us one, otherwise the default window —
  // but never sooner than MIN_TRANSIENT_RETRY_MS so a `Retry-After: 0` cannot
  // spin the expiry timer into a tight refetch loop.
  if (outcome === "transient") {
    return now + Math.max(MIN_TRANSIENT_RETRY_MS, transientRetryMs);
  }
  // A permanent rejection (SSRF/policy) is a hard miss: retrying only repeats
  // the same rejected work, so hold it for the full miss TTL like a null result.
  if (outcome === "rejected" || metadata === null) return now + NULL_METADATA_RETRY_MS;
  if (metadata.imageFetchState !== "transient_failure") return null;
  const retryAfterMs =
    typeof metadata.imageRetryAfterMs === "number" &&
    Number.isFinite(metadata.imageRetryAfterMs)
      ? Math.max(MIN_TRANSIENT_RETRY_MS, metadata.imageRetryAfterMs)
      : DEFAULT_TRANSIENT_RETRY_MS;
  return now + retryAfterMs;
}

function createTaskScheduler(concurrency: number) {
  const pending: Array<() => void> = [];
  let active = 0;

  const drain = () => {
    while (active < concurrency) {
      const run = pending.shift();
      if (!run) return;
      active += 1;
      run();
    }
  };

  return <T>(task: () => Promise<T>): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      pending.push(() => {
        void task()
          .then(resolve, reject)
          .finally(() => {
            active -= 1;
            drain();
          });
      });
      drain();
    });
}

function createMetadataLoader({
  concurrency = MAX_CONCURRENT_METADATA_FETCHES,
  delay = (ms: number) =>
    new Promise<void>((resolve) => setTimeout(resolve, ms)),
  fetcher,
  now = Date.now,
}: {
  concurrency?: number;
  delay?: (ms: number) => Promise<void>;
  fetcher: (href: string) => Promise<LinkPreviewMetadata | null>;
  now?: () => number;
}) {
  const cache = new Map<
    string,
    MetadataCacheEntry | Promise<MetadataLoadResult>
  >();
  const schedule = createTaskScheduler(Math.max(1, concurrency));
  let generation = 0;

  const peek = (href: string): MetadataLoadResult | undefined => {
    const key = metadataCacheKey(href);
    const cached = cache.get(key);
    if (!cached || cached instanceof Promise) return undefined;
    if (cached.expiresAt !== null && cached.expiresAt <= now()) {
      cache.delete(key);
      return undefined;
    }
    return { key, ...cached };
  };

  const load = (href: string): Promise<MetadataLoadResult> => {
    const key = metadataCacheKey(href);
    const cached = cache.get(key);
    if (cached instanceof Promise) return cached;
    if (cached) {
      if (cached.expiresAt === null || cached.expiresAt > now()) {
        return Promise.resolve({ key, ...cached });
      }
      cache.delete(key);
    }

    const requestGeneration = generation;
    // One coalesced attempt sequence. A transient blip earns a single inline
    // retry after a short backoff before we give up and fall into the longer
    // self-heal wait; a rate limit (429) skips that retry — an immediate retry
    // would just be throttled again — and waits out its Retry-After instead; a
    // permanent rejection (SSRF/policy) skips every retry and caches as a hard
    // miss. Each network fetch is scheduled separately and the backoff sleeps
    // OUTSIDE the scheduler, so a failing URL never holds a concurrency slot
    // through its wait to starve healthy previews.
    const succeeded = (metadata: LinkPreviewMetadata | null): AttemptResult => ({
      metadata,
      outcome: "ok",
      transientRetryMs: DEFAULT_TRANSIENT_RETRY_MS,
    });
    const failed = (failure: FetchFailure): AttemptResult => ({
      metadata: null,
      outcome: failure.kind === "rejected" ? "rejected" : "transient",
      transientRetryMs:
        failure.kind === "rate_limited"
          ? (failure.retryAfterMs ?? DEFAULT_TRANSIENT_RETRY_MS)
          : DEFAULT_TRANSIENT_RETRY_MS,
    });
    const scheduledFetch = (): Promise<LinkPreviewMetadata | null> =>
      schedule(() => fetcher(href));

    const attempt = (): Promise<AttemptResult> =>
      scheduledFetch().then(succeeded, (reason) => {
        const failure = classifyFetchFailure(reason);
        // Only an ordinary transient blip earns the inline retry; a rejection
        // is permanent and a rate limit must wait, so both resolve immediately.
        if (failure.kind !== "transient") return failed(failure);
        return delay(INLINE_RETRY_BACKOFF_MS).then(() =>
          // Re-classify the retry's own failure so a 429 (or a rejection) on the
          // second attempt keeps its Retry-After / hard-miss semantics instead
          // of collapsing to the generic transient window.
          scheduledFetch().then(succeeded, (retryReason) =>
            failed(classifyFetchFailure(retryReason)),
          ),
        );
      });

    const promise = attempt().then(
      ({ metadata, outcome, transientRetryMs }) => {
        const entry = {
          expiresAt: metadataExpiry(
            metadata,
            now(),
            outcome,
            transientRetryMs,
          ),
          metadata,
        };
        if (requestGeneration === generation) {
          cache.set(key, entry);
        }
        return { key, ...entry };
      },
    );
    cache.set(key, promise);
    return promise;
  };

  return {
    deleteKey(key: string) {
      cache.delete(key);
    },
    load,
    peek,
    reset() {
      generation += 1;
      cache.clear();
    },
  };
}

function fetchLinkPreviewMetadata(
  href: string,
): Promise<LinkPreviewMetadata | null> {
  return invokeTauri<LinkPreviewMetadata | null>(
    "fetch_link_preview_metadata",
    {
      href,
    },
  );
}

const metadataLoader = createMetadataLoader({
  fetcher: fetchLinkPreviewMetadata,
});
const ENTITY_STATUS_KINDS = [
  KIND_GIT_STATUS_OPEN,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
];

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

  const trustedAuthors = [...new Set([root.pubkey.toLowerCase(), owner])];
  const statusEvents = await fetchEvents({
    kinds: ENTITY_STATUS_KINDS,
    authors: trustedAuthors,
    "#e": [id],
    limit: 20,
  });
  if (type === "issue") {
    const issue = eventToProjectIssue(root, statusEvents);
    return {
      ...base,
      title: issue.title,
      description: compactMetadata([issue.status, ...issue.labels.slice(0, 2)]),
    };
  }

  const pullRequest = eventToProjectPullRequest(root, [], [], statusEvents);
  const source = pullRequest.branchName;
  const target = pullRequest.targetBranch ?? repository.defaultBranch;
  return {
    ...base,
    title: pullRequest.title,
    description: compactMetadata([
      pullRequest.status,
      source ? `${source} → ${target}` : null,
      pullRequest.commit?.slice(0, 7),
    ]),
  };
}

const entityMetadataLoader = createMetadataLoader({
  fetcher: fetchBuzzEntityMetadata,
});

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
  if (preview.kind !== "buzz-pull-request" && preview.kind !== "buzz-issue") {
    return true;
  }
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
    preview.kind === "buzz-repository"
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
): ResolvedLinkPreview[] {
  const [resolvedMetadata, setResolvedMetadata] =
    React.useState<ResolvedMetadataByHref>({});
  const [retryGeneration, setRetryGeneration] = React.useState(0);

  React.useEffect(() => {
    let cancelled = false;
    let retryAt = Number.POSITIVE_INFINITY;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

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
          void loader.load(preview.href).then((result) => {
            if (cancelled) return;
            setResolvedMetadata((current) =>
              current[result.key] === result.metadata
                ? current
                : { ...current, [result.key]: result.metadata },
            );
            scheduleRetry(result, loader);
          });
        }),
      );
    }

    return () => {
      cancelled = true;
      for (const cancel of cancelScheduledLoads) cancel();
      if (retryTimer !== null) clearTimeout(retryTimer);
    };
  }, [previews, retryGeneration]);

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
