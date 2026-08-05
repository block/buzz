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
  fetcher,
  now = Date.now,
}: {
  concurrency?: number;
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
    const promise = schedule(() => fetcher(href))
      .catch(() => null)
      .then((metadata) => {
        const entry = {
          expiresAt: metadataExpiry(metadata, now()),
          metadata,
        };
        if (requestGeneration === generation) {
          cache.set(key, entry);
        }
        return { key, ...entry };
      });
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
const entityTitleLoader = createMetadataLoader({
  fetcher: async (href) => {
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
    if (
      !event?.tags.some(
        (tag) => tag[0] === "a" && tag[1] === expectedCoordinate,
      )
    ) {
      return null;
    }

    const subject = event.tags.find((tag) => tag[0] === "subject")?.[1];
    const title = subject || event.content.split("\n")[0] || null;
    return title
      ? {
          title,
          siteName: "Buzz",
          description: null,
          imageDataUrl: null,
          imageDomain: null,
        }
      : null;
  },
});

/** Clear ephemeral metadata when the active relay/community changes. */
export function resetLinkPreviewMetadataCache(): void {
  metadataLoader.reset();
  entityTitleLoader.reset();
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
    return { ...preview, imageState: "pending" };
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
      preview.kind === "generic-link" && metadata.siteName
        ? metadata.siteName
        : preview.provider,
    imageDataUrl: hasImage ? metadata.imageDataUrl : null,
    imageDomain: hasImage ? metadata.imageDomain : null,
    imageState,
  };
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
        ? entityTitleLoader
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
