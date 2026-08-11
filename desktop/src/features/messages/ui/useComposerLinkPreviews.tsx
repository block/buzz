import * as React from "react";
import { ImageOff, LoaderCircle, X } from "lucide-react";
import { toast } from "sonner";

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

// Idle time after the last keystroke before link-preview resolution runs, so
// typing a URL does not flicker a card per character (debounce, not throttle:
// throttle would still fire mid-type).
const LINK_PREVIEW_DEBOUNCE_MS = 350;

// Upper bound on how long Send stays disabled while a preview is still settling
// (metadata resolving, or snapshot media uploading). Past this the button
// re-enables even if the tag never lands, so a dead or slow link never traps
// the composer — the message then sends as a bare link.
const SNAPSHOT_SETTLE_DISABLE_CAP_MS = 2000;

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
// "none" marker. Live hrefs without a ready tag (dead/slow link past the
// anti-trap cap) are omitted and the message sends as a bare link.
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
  preview,
  tagReady,
}: {
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
      data-snapshot-tag-ready={snapshotTagReady ? "true" : "false"}
      state={done ? "done" : "processing"}
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
          {done ? preview.title : hostname}
        </AttachmentTitle>
        <AttachmentDescription>
          {done
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

// Upload one snapshot media (image or favicon) independently so a single
// failure degrades gracefully instead of dropping the whole preview: on
// failure we return empty url/sha256 (a valid "no media" snapshot field) and
// report which media failed so the caller can toast the user once.
async function uploadSnapshotMedia(
  dataUrl: string | null | undefined,
  filename: string,
  label: "thumbnail" | "favicon",
): Promise<{ url: string; sha256: string; failed: null | typeof label }> {
  try {
    const { url, sha256 } = await uploadDataUrl(dataUrl, filename);
    return { url, sha256, failed: null };
  } catch {
    return { url: "", sha256: "", failed: dataUrl ? label : null };
  }
}

export function useComposerLinkPreviews(content: string, enabled = true) {
  const [suppressed, setSuppressed] = React.useState(false);
  // Debounce the content that drives resolution so typing a URL character by
  // character does not churn a new candidate href (and a flickering card) per
  // keystroke. `content` is the live editor value; `debounced` is what actually
  // resolves. A fast paste-and-Enter before the debounce fires is held by
  // `hasUnresolvedLiveCandidates` below, which keeps Send disabled until the
  // live candidates resolve — so no synchronous flush is needed at submit.
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
  // Supported candidates in the LIVE content. When these differ from what has
  // resolved (debounce not yet fired after a paste/keystroke), Send must still
  // treat the preview as pending so a fast Enter cannot ship a bare link ahead
  // of resolution.
  const liveCandidatesRef = React.useRef<string[]>([]);
  liveCandidatesRef.current = extractCandidates(content).map(
    (preview) => preview.href,
  );
  const resolvedPreviews = useResolvedLinkPreviews(
    suppressed ? [] : candidates,
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
  const liveCandidatesEmpty = liveCandidatesRef.current.length === 0;
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
      // Upload image and favicon independently so one failure degrades to the
      // surviving media instead of dropping the whole preview. A snapshot tag
      // with empty media fields is valid (renders as text + favicon, or
      // text-only), so a partial or total media failure still ships a real
      // inline preview and the card never spins forever.
      const uploadPromise = Promise.all([
        uploadSnapshotMedia(
          preview.imageDataUrl,
          "link-preview-image.png",
          "thumbnail",
        ),
        uploadSnapshotMedia(
          preview.faviconDataUrl,
          "link-preview-favicon.png",
          "favicon",
        ),
      ])
        .then(([image, favicon]) => {
          if (!activeHrefsRef.current.has(preview.href)) return;
          const failedMedia = [image.failed, favicon.failed].filter(
            (label): label is "thumbnail" | "favicon" => label !== null,
          );
          if (failedMedia.length > 0) {
            toast.error(
              `Something went wrong with the ${failedMedia.join(" and ")}`,
            );
          }
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
          // Update the ref alongside state so a submit reading
          // `readyTagsByHrefRef` sees the tag before the next render commits.
          readyTagsByHrefRef.current = {
            ...readyTagsByHrefRef.current,
            [preview.href]: tag,
          };
          setReadyTags((current) => ({ ...current, [preview.href]: tag }));
        })
        .finally(() => {
          uploadsRef.current.delete(preview.href);
        });
      void uploadPromise;
    }
  }, [previews, readyTags]);

  readyTagsRef.current = suppressed
    ? [["link-preview", "none"]]
    : candidates.flatMap((candidate) =>
        readyTags[candidate.href] ? [readyTags[candidate.href]] : [],
      );
  // A preview is "settling" from paste until its sendable tag exists: metadata
  // is still resolving, or it resolved and the snapshot media is uploading.
  // Send stays disabled across the whole window so the button never flickers
  // ready -> not-ready -> ready (buzz:// links never snapshot, so they never
  // report settling). `imageState === "none"` is terminal (no snapshot), so it
  // does not block. See the disable cap below for the dead/slow-link escape.
  const hasResolvingSnapshots =
    !suppressed &&
    previews.some(
      (preview) =>
        !preview.href.startsWith("buzz://") &&
        (preview.imageState === "pending" ||
          (preview.snapshotReady && !readyTags[preview.href])),
    );
  // A supported link in the LIVE content that resolution has not caught up to
  // yet (debounce pending, or resolved for an older revision) also counts as
  // settling — otherwise a paste-and-immediate-Enter would ship a bare link
  // before resolution even starts. buzz:// links never snapshot, so ignore them.
  const hasUnresolvedLiveCandidates =
    !suppressed &&
    liveCandidatesRef.current.some(
      (href) =>
        !href.startsWith("buzz://") &&
        !readyTags[href] &&
        !candidates.some((candidate) => candidate.href === href),
    );
  const hasSettlingSnapshots =
    hasResolvingSnapshots || hasUnresolvedLiveCandidates;
  // Re-enable Send once the disable cap elapses even if a preview is still
  // settling, so a link whose metadata or upload stalls never traps the
  // composer. Resets whenever settling ends or the live candidate set changes.
  const [settleDisableExpired, setSettleDisableExpired] = React.useState(false);
  const liveCandidatesKey = liveCandidatesRef.current.join("\n");
  // biome-ignore lint/correctness/useExhaustiveDependencies: liveCandidatesKey intentionally restarts the anti-trap cap when the link set changes while still settling, so a replaced/added link gets a fresh disable window rather than inheriting the prior link's near-expired timer.
  React.useEffect(() => {
    if (!hasSettlingSnapshots) {
      setSettleDisableExpired(false);
      return;
    }
    setSettleDisableExpired(false);
    const timer = window.setTimeout(
      () => setSettleDisableExpired(true),
      SNAPSHOT_SETTLE_DISABLE_CAP_MS,
    );
    return () => window.clearTimeout(timer);
  }, [hasSettlingSnapshots, liveCandidatesKey]);
  const hasPendingSnapshots = hasSettlingSnapshots && !settleDisableExpired;
  // Ref mirror so a synchronous submit guard can read the pending state on any
  // entry point (Enter, form, auto-submit), not just the reactive button prop.
  const hasPendingSnapshotsRef = React.useRef(hasPendingSnapshots);
  hasPendingSnapshotsRef.current = hasPendingSnapshots;
  const hideAll = React.useCallback(() => setSuppressed(true), []);
  const previewList = previews.length ? (
    <div
      className="mb-2"
      data-composer-link-previews=""
      data-has-pending-snapshots={hasPendingSnapshots ? "true" : "false"}
      data-ready-snapshot-count={readyTagsRef.current.length}
    >
      <div className="flex max-w-full items-start gap-1">
        <AttachmentGroup className="max-w-full flex-row flex-wrap items-start overflow-visible pb-0">
          {previews.map((preview) => (
            <ComposerLinkPreviewCard
              key={preview.href}
              preview={preview}
              tagReady={Boolean(readyTags[preview.href])}
            />
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
  // Snapshot tags for a submit, read synchronously at submit start from the
  // LIVE candidate set (liveCandidatesRef) via `selectSubmitTags` — so the tags
  // always correspond to the content actually being sent, never a debounced set
  // that still holds a just-removed URL. No await: Send is disabled until every
  // settling preview has its tag (or the anti-trap cap fires), so at submit time
  // the tags that will ever exist already exist.
  const getReadyTags = React.useCallback(
    () =>
      selectSubmitTags(
        liveCandidatesRef.current,
        readyTagsByHrefRef.current,
        suppressedRef.current,
      ),
    [],
  );
  return {
    previewList,
    getReadyTags,
    hasPendingSnapshots,
    hasPendingSnapshotsRef,
  };
}
