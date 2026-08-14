import * as React from "react";
import { ImageOff, X } from "lucide-react";

import { getRelayHttpUrl } from "@/shared/api/tauri";
import { extractSupportedLinkPreviews } from "@/shared/lib/linkPreview";
import { isValidLinkPreviewSnapshotCanonicalUrl } from "@/shared/lib/linkPreviewSnapshot";
import {
  invalidateLinkPreviewPreparation,
  prepareLinkPreview,
} from "@/features/messages/lib/linkPreviewPreparationStore";
import {
  beginRelayOriginFetch,
  getCachedRelayOrigin,
} from "@/shared/lib/mediaUrl";
import {
  isBuzzEntityPreview,
  type ResolvedLinkPreview,
  useResolvedLinkPreviews,
  withEntityFallbacks,
} from "@/shared/lib/useResolvedLinkPreviews";
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
import { Progress } from "@/shared/ui/progress";
import { Skeleton } from "@/shared/ui/skeleton";

// Idle time after the last keystroke before link-preview resolution runs, so
// typing a URL does not flicker a card per character (debounce, not throttle:
// throttle would still fire mid-type).
const LINK_PREVIEW_DEBOUNCE_MS = 350;

// A preview stays pending until its metadata and snapshot media settle. The
// visible suppression control is the explicit escape for sending without
// previews; network timing must never silently change the submitted event.

function previewHostname(href: string): string {
  try {
    return new URL(href).hostname.replace(/^www\./, "");
  } catch {
    return href;
  }
}

// Pure selector for the snapshot tags emitted on submit. Keyed off `liveHrefs`
// (the hrefs in the content being sent RIGHT NOW), never the debounced active
// set — a ready tag for URL A lingers in `tagsByHref` for the 350 ms until the
// debounce drops A, so keying off live hrefs is what stops "delete A, send
// replacement text within the window" from leaking A's tag (and media refs)
// onto a body that no longer contains A. When `suppressed`, emit only the
// "none" marker. An unsuppressed live href without a ready tag can only reach
// submit after a terminal miss; it is omitted and sends as a bare link.
export function selectSubmitTags(
  liveHrefs: readonly string[],
  tagsByHref: Record<string, string[]>,
  suppressed: boolean,
): string[][] {
  if (suppressed) return [["link-preview", "none"]];
  return liveHrefs.flatMap((href) => {
    const tag = tagsByHref[href];
    return tag ? [tag] : [];
  });
}

function ComposerLinkPreviewCard({
  onSuppress,
  preview,
  tagReady,
}: {
  onSuppress: () => void;
  preview: ResolvedLinkPreview;
  tagReady: boolean;
}) {
  const imageSrc = preview.imageState === "image" ? preview.imageDataUrl : null;
  const [failedImageSrc, setFailedImageSrc] = React.useState<string | null>(
    null,
  );
  const showImage = Boolean(imageSrc && failedImageSrc !== imageSrc);
  const hostname = previewHostname(preview.href);
  // External cards are send-ready only once their snapshot tag exists. Buzz
  // entities never snapshot; recipients resolve them from the relay, so they
  // are complete as soon as the recognized entity card exists.
  const snapshotTagReady = Boolean(preview.snapshotReady && tagReady);
  const done = snapshotTagReady || isBuzzEntityPreview(preview);

  return (
    <div
      className="group/link-preview relative w-80 max-w-full"
      data-image-state={preview.imageState}
      data-link-preview={preview.kind}
      data-link-preview-composer-card=""
      data-snapshot-tag-ready={snapshotTagReady ? "true" : "false"}
    >
      <Attachment
        className="h-[55px] w-full gap-0 overflow-hidden p-0 pr-2 shadow-none"
        state={done ? "done" : "processing"}
      >
        <AttachmentMedia
          className="relative h-[55px] w-[55px] rounded-none rounded-l-2xl border-0 bg-muted"
          data-link-preview-thumbnail=""
          variant="image"
        >
          {showImage ? (
            <img
              alt=""
              className={`h-full w-full object-cover ${done ? "" : "opacity-50"}`}
              onError={() => setFailedImageSrc(imageSrc ?? null)}
              src={imageSrc ?? undefined}
            />
          ) : !done ? (
            <div
              className="h-full w-full animate-pulse bg-muted"
              data-testid="link-preview-thumbnail-placeholder"
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
          {!done ? (
            <div className="absolute inset-x-0 bottom-0 px-1.5 pb-1.5">
              <Progress
                aria-label="Loading link preview"
                className="h-1 bg-foreground/15 [&>div]:bg-foreground/80"
                data-testid="link-preview-progress"
                value={null}
              />
            </div>
          ) : null}
        </AttachmentMedia>
        <AttachmentContent className="pl-2">
          {done ? (
            <>
              <AttachmentTitle
                className="line-clamp-1"
                data-link-preview-hostname=""
              >
                {preview.title}
              </AttachmentTitle>
              <AttachmentDescription>
                {preview.provider || hostname}
              </AttachmentDescription>
            </>
          ) : (
            <div
              aria-label="Loading link preview details"
              className="space-y-2"
              data-testid="link-preview-text-placeholder"
              role="status"
            >
              <Skeleton className="h-3.5 w-3/4" />
              <Skeleton className="h-2.5 w-1/2" />
            </div>
          )}
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
      <Button
        aria-label="Send without link previews"
        className="absolute -right-1 -top-1 z-20 flex h-4 w-4 items-center justify-center rounded-full bg-foreground p-0 text-background opacity-0 shadow-none transition-opacity hover:bg-foreground group-hover/link-preview:opacity-100 group-focus-within/link-preview:opacity-100 focus-visible:opacity-100 [&_svg]:size-2.5"
        data-testid="composer-hide-link-previews"
        onClick={onSuppress}
        size="icon-xs"
        title="Send without link previews"
        type="button"
      >
        <X aria-hidden="true" />
      </Button>
    </div>
  );
}

export interface ComposerLinkPreviewInput {
  content: string;
  hrefs: Set<string>;
  hrefVersions: Map<string, number>;
  nextHrefVersion: number;
}

export function updateComposerLinkPreviewInput(
  current: ComposerLinkPreviewInput,
  content: string,
): ComposerLinkPreviewInput {
  const nextHrefs = new Set(
    extractSupportedLinkPreviews(content)
      .filter((preview) =>
        preview.href.startsWith("buzz://")
          ? true
          : isValidLinkPreviewSnapshotCanonicalUrl(preview.href),
      )
      .map((preview) => preview.href),
  );
  const nextVersions = new Map<string, number>();
  let nextHrefVersion = current.nextHrefVersion;
  for (const href of nextHrefs) {
    if (current.hrefs.has(href)) {
      const version = current.hrefVersions.get(href);
      if (version !== undefined) nextVersions.set(href, version);
      continue;
    }
    nextHrefVersion += 1;
    nextVersions.set(href, nextHrefVersion);
  }
  return {
    content,
    hrefs: nextHrefs,
    hrefVersions: nextVersions,
    nextHrefVersion,
  };
}

export function useComposerLinkPreviewInput() {
  const [input, setInput] = React.useState<ComposerLinkPreviewInput>(() => ({
    content: "",
    hrefs: new Set(),
    hrefVersions: new Map(),
    nextHrefVersion: 0,
  }));
  const update = React.useCallback(
    (content: string) =>
      setInput((current) => updateComposerLinkPreviewInput(current, content)),
    [],
  );
  return [input, update] as const;
}

export function useManagedComposerLinkPreviews(enabled = true) {
  const [input, updateContent] = useComposerLinkPreviewInput();
  return {
    ...useComposerLinkPreviews(input.content, enabled, input.hrefVersions),
    updateContent,
  };
}

export function useComposerLinkPreviews(
  content: string,
  enabled = true,
  liveHrefVersions?: ReadonlyMap<string, number>,
) {
  const [suppressed, setSuppressed] = React.useState(false);
  // Debounce the content that drives resolution so typing a URL character by
  // character does not churn a new candidate href (and a flickering card) per
  // keystroke. `content` is the live editor value; `debounced` drives the
  // speculative composer card. Submit independently freezes live candidates,
  // so a fast paste-and-Enter promotes the exact URL without waiting here.
  const [debounced, setDebounced] = React.useState(content);
  const debouncedRef = React.useRef(debounced);
  debouncedRef.current = debounced;
  React.useEffect(() => {
    if (content === debouncedRef.current) return;
    const timer = window.setTimeout(
      () => setDebounced(content),
      LINK_PREVIEW_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [content]);
  const extractCandidates = React.useCallback(
    (source: string) =>
      enabled
        ? extractSupportedLinkPreviews(source).filter((preview) =>
            preview.href.startsWith("buzz://")
              ? true
              : isValidLinkPreviewSnapshotCanonicalUrl(preview.href),
          )
        : [],
    [enabled],
  );
  const candidates = React.useMemo(
    () => extractCandidates(debounced),
    [extractCandidates, debounced],
  );
  const liveCandidates = React.useMemo(
    () => extractCandidates(content).map((preview) => preview.href),
    [content, extractCandidates],
  );
  const liveCandidatesKey = liveCandidates.join("\n");
  const liveHrefVersionsKey = liveCandidates
    .map((href) => `${href}\0${liveHrefVersions?.get(href) ?? ""}`)
    .join("\n");
  // Async completion and submit paths may only observe committed editor state.
  // A render-time ref write can leak an abandoned concurrent render.
  const liveCandidatesRef = React.useRef<string[]>([]);
  React.useLayoutEffect(() => {
    liveCandidatesRef.current = liveCandidates;
  }, [liveCandidates]);
  const resolvedPreviews = useResolvedLinkPreviews(
    suppressed ? [] : candidates,
    {
      refetchNewNegatives: true,
      liveHrefs: liveCandidates,
      liveHrefVersions,
    },
  );
  // Entity links resolve to null metadata when the relay lookup has nothing
  // for them; keep their safe fallback cards rather than dropping them.
  const previews = React.useMemo(
    () => withEntityFallbacks(suppressed ? [] : candidates, resolvedPreviews),
    [suppressed, candidates, resolvedPreviews],
  );
  // Clear a "hide previews" suppression as soon as the LIVE draft has no
  // supported candidates — not the debounced set, whose lag would otherwise let
  // a clear-then-retype race keep suppression stuck on after the draft changed.
  const liveCandidatesEmpty = liveCandidates.length === 0;
  React.useEffect(() => {
    if (liveCandidatesEmpty) setSuppressed(false);
  }, [liveCandidatesEmpty]);
  const [readyTags, setReadyTags] = React.useState<Record<string, string[]>>(
    {},
  );
  const readyTagsRef = React.useRef<string[][]>([]);
  const readyTagsByHrefRef = React.useRef(readyTags);
  readyTagsByHrefRef.current = readyTags;
  const suppressedRef = React.useRef(suppressed);
  suppressedRef.current = suppressed;
  // Track composer incarnations independently from the shared preparation
  // store. Re-entry must supersede work built from the previous incarnation,
  // even if React batched leave+enter into one commit.
  const preparationGenerationRef = React.useRef(new Map<string, number>());
  const committedLiveHrefsRef = React.useRef<Set<string>>(new Set());
  const committedLiveHrefVersionsRef = React.useRef<Map<string, number>>(
    new Map(),
  );
  const reenteringHrefsRef = React.useRef<
    Map<string, "blocked" | "refetching">
  >(new Map());
  const activeHrefsRef = React.useRef(new Set<string>());
  const reenteredLiveHrefs = liveCandidates.filter((href) => {
    const version = liveHrefVersions?.get(href);
    return version === undefined
      ? !committedLiveHrefsRef.current.has(href)
      : committedLiveHrefVersionsRef.current.get(href) !== version;
  });
  const staleReenteredHrefs = reenteredLiveHrefs.filter(
    (href) =>
      !reenteringHrefsRef.current.has(href) &&
      previews.some(
        (preview) =>
          preview.href === href &&
          preview.snapshotReady &&
          preview.imageState === "fallback",
      ),
  );
  const staleReenteredKey = staleReenteredHrefs.join("\n");

  // biome-ignore lint/correctness/useExhaustiveDependencies: stable keys represent the committed live/version/stale sets.
  React.useLayoutEffect(() => {
    const previous = committedLiveHrefsRef.current;
    const active = new Set(liveCandidates);
    activeHrefsRef.current = active;
    committedLiveHrefsRef.current = active;
    committedLiveHrefVersionsRef.current = new Map(liveHrefVersions);

    for (const href of previous) {
      if (active.has(href)) continue;
      reenteringHrefsRef.current.delete(href);
      preparationGenerationRef.current.set(
        href,
        (preparationGenerationRef.current.get(href) ?? 0) + 1,
      );
    }

    if (staleReenteredHrefs.length === 0) return;
    for (const href of staleReenteredHrefs) {
      reenteringHrefsRef.current.set(href, "blocked");
      preparationGenerationRef.current.set(
        href,
        (preparationGenerationRef.current.get(href) ?? 0) + 1,
      );
      invalidateLinkPreviewPreparation(href);
    }
    const drop = new Set(staleReenteredHrefs);
    setReadyTags((current) => {
      let changed = false;
      const next = { ...current };
      for (const href of drop) {
        if (!(href in next)) continue;
        delete next[href];
        changed = true;
      }
      return changed ? next : current;
    });
  }, [liveCandidatesKey, liveHrefVersionsKey, staleReenteredKey]);

  const isHrefReentering = (href: string) =>
    reenteringHrefsRef.current.has(href) || staleReenteredHrefs.includes(href);

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
      const phase = reenteringHrefsRef.current.get(preview.href);
      if (phase !== undefined) {
        if (!preview.snapshotReady) {
          reenteringHrefsRef.current.set(preview.href, "refetching");
          continue;
        }
        if (phase === "blocked") continue;
        reenteringHrefsRef.current.delete(preview.href);
      }
      if (!preview.snapshotReady || readyTags[preview.href]) continue;
      const generation =
        preparationGenerationRef.current.get(preview.href) ?? 0;
      void prepareLinkPreview(preview).then((tag) => {
        if (
          !tag ||
          suppressedRef.current ||
          !activeHrefsRef.current.has(preview.href) ||
          reenteringHrefsRef.current.has(preview.href) ||
          (preparationGenerationRef.current.get(preview.href) ?? 0) !==
            generation
        ) {
          return;
        }
        readyTagsByHrefRef.current = {
          ...readyTagsByHrefRef.current,
          [preview.href]: tag,
        };
        setReadyTags((current) => ({ ...current, [preview.href]: tag }));
      });
    }
  }, [previews, readyTags]);

  readyTagsRef.current = suppressed
    ? [["link-preview", "none"]]
    : candidates.flatMap((candidate) =>
        readyTags[candidate.href] && !isHrefReentering(candidate.href)
          ? [readyTags[candidate.href]]
          : [],
      );
  // Expose speculative preparation state for card treatment and tests. It no
  // longer gates Submit: the send flow promotes unfinished work into the
  // navigation-safe preparation store.
  const hasResolvingSnapshots =
    !suppressed &&
    previews.some(
      (preview) =>
        !preview.href.startsWith("buzz://") &&
        (preview.imageState === "pending" ||
          isHrefReentering(preview.href) ||
          (preview.snapshotReady && !readyTags[preview.href])),
    );
  // Include live candidates not reached by the debounce yet so the composer
  // accurately reports whether its visible generation is still catching up.
  const hasUnresolvedLiveCandidates =
    !suppressed &&
    liveCandidates.some(
      (href) =>
        !href.startsWith("buzz://") &&
        !readyTags[href] &&
        !candidates.some((candidate) => candidate.href === href),
    );
  const hasPendingSnapshots =
    hasResolvingSnapshots || hasUnresolvedLiveCandidates;
  // Ref mirror retained for consumers that need an imperative status read.
  const hasPendingSnapshotsRef = React.useRef(hasPendingSnapshots);
  hasPendingSnapshotsRef.current = hasPendingSnapshots;
  const hideAll = React.useCallback(() => {
    setSuppressed(true);
  }, []);
  const previewList = previews.length ? (
    <div
      className="mb-2"
      data-composer-link-previews=""
      data-has-pending-snapshots={hasPendingSnapshots ? "true" : "false"}
      data-ready-snapshot-count={readyTagsRef.current.length}
    >
      <AttachmentGroup className="max-w-full flex-row flex-wrap items-start overflow-visible pb-0">
        {previews.map((preview) => (
          <ComposerLinkPreviewCard
            key={preview.href}
            onSuppress={hideAll}
            preview={preview}
            tagReady={Boolean(readyTags[preview.href])}
          />
        ))}
      </AttachmentGroup>
    </div>
  ) : null;
  // Snapshot tags already available at submit, selected from the LIVE candidate
  // set so a debounced, just-removed URL can never leak into the event. The send
  // flow promotes any missing candidates and ignores this partial set.
  const getReadyTags = React.useCallback(
    () =>
      selectSubmitTags(
        liveCandidatesRef.current.filter(
          (href) => !reenteringHrefsRef.current.has(href),
        ),
        readyTagsByHrefRef.current,
        suppressedRef.current,
      ),
    [],
  );
  const getLiveCandidates = React.useCallback(
    () => extractCandidates(content),
    [content, extractCandidates],
  );
  return {
    previewList,
    getLiveCandidates,
    getReadyTags,
    hasPendingSnapshots,
    hasPendingSnapshotsRef,
  };
}
