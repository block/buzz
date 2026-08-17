import * as React from "react";

import {
  type BlobDescriptor,
  pickAndUploadMedia,
  uploadMediaBytes,
} from "@/shared/api/tauri";
import { uploadMediaFile } from "@/shared/api/tauriMedia";
import type { QueuedMediaAttachment } from "./backgroundMediaUploadStore";
import { applyImetaUpdate, compactImetaSlots } from "./imetaSlots";
import { useFilePicker } from "./useFilePicker";
import { isVideoFile, videoMimeForFile } from "./videoFileType";

/**
 * First 4 hex chars of the sha256 — used as a short display name.
 * Note: 4 hex chars = 65,536 possible values. Collision is unlikely
 * within a single message's attachments but theoretically possible.
 * If collisions become an issue, extend to 6+ chars.
 */
export function shortHash(sha256: string): string {
  return sha256.slice(0, 4);
}

type UploadState = {
  status: "idle" | "uploading" | "error";
  message?: string;
};

export type UploadingAttachmentPreview = {
  id: number;
  dim?: string;
  filename?: string;
  posterUrl?: string;
  /** Upload progress 0–100, or null while no byte counts exist yet
   * (e.g. video transcoding before the HTTP upload starts). */
  progress?: number | null;
  slotIndex?: number;
  spoilered?: boolean;
  type?: string;
  /**
   * Upload epoch this preview was created in. Cancel handling compares it
   * against the current epoch so a preview left over from a replaced draft
   * cannot null a slot belonging to the draft now on screen.
   */
  uploadEpoch?: number;
};

/** Correlation id for the Rust `media-upload-progress` events. */
function uploadProgressId(previewId: number): string {
  return `composer-upload-${previewId}`;
}

/** True when the drag payload contains files (not plain text or URLs). */
function isFileDrag(event: React.DragEvent<HTMLElement>): boolean {
  return event.dataTransfer?.types.includes("Files") ?? false;
}

function waitForMediaEvent(
  element: HTMLMediaElement,
  eventName: string,
  timeoutMs: number,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const timeoutId = window.setTimeout(() => {
      cleanup();
      reject(new Error(`Timed out waiting for ${eventName}`));
    }, timeoutMs);

    function cleanup() {
      window.clearTimeout(timeoutId);
      element.removeEventListener(eventName, onEvent);
      element.removeEventListener("error", onError);
    }

    function onEvent() {
      cleanup();
      resolve();
    }

    function onError() {
      cleanup();
      reject(new Error(`Could not load media for ${eventName}`));
    }

    element.addEventListener(eventName, onEvent, { once: true });
    element.addEventListener("error", onError, { once: true });
  });
}

type CapturedVideoPoster = {
  dim: string;
  posterUrl: string;
};

async function captureVideoPosterFrame(
  file: File,
): Promise<CapturedVideoPoster | null> {
  const videoMime = videoMimeForFile(file);
  if (!videoMime) return null;

  // A blob URL inherits the File's own MIME type, so a video whose type is
  // empty or `application/octet-stream` would be rejected by the <video>
  // element and yield no poster. Re-type the bytes with the MIME we derived
  // from the extension; `slice` wraps the same bytes without copying them.
  const objectUrl = URL.createObjectURL(
    file.type === videoMime ? file : file.slice(0, file.size, videoMime),
  );
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.preload = "metadata";

  try {
    video.src = objectUrl;
    await waitForMediaEvent(video, "loadedmetadata", 3_000);

    const duration = Number.isFinite(video.duration) ? video.duration : 0;
    const seekTime = duration > 0.2 ? 0.1 : 0;
    if (seekTime > 0) {
      const seeked = waitForMediaEvent(video, "seeked", 2_000);
      video.currentTime = seekTime;
      await seeked.catch(() => undefined);
    } else if (video.readyState < 2) {
      await waitForMediaEvent(video, "loadeddata", 2_000).catch(
        () => undefined,
      );
    }

    await new Promise((resolve) => requestAnimationFrame(resolve));

    if (video.videoWidth === 0 || video.videoHeight === 0) return null;

    const maxWidth = 640;
    const scale = Math.min(1, maxWidth / video.videoWidth);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(video.videoWidth * scale));
    canvas.height = Math.max(1, Math.round(video.videoHeight * scale));

    const context = canvas.getContext("2d");
    if (!context) return null;
    context.drawImage(video, 0, 0, canvas.width, canvas.height);
    return {
      dim: `${video.videoWidth}x${video.videoHeight}`,
      posterUrl: canvas.toDataURL("image/jpeg", 0.82),
    };
  } catch {
    return null;
  } finally {
    URL.revokeObjectURL(objectUrl);
    video.removeAttribute("src");
    video.load();
  }
}

type UseMediaUploadOptions = {
  /** Keep newly selected videos local until the message is submitted. */
  deferUploadsUntilSend?: boolean;
};

export function useMediaUpload({
  deferUploadsUntilSend = false,
}: UseMediaUploadOptions = {}) {
  const e2eConfig = (
    window as Window & {
      __BUZZ_E2E__?: { mock?: { deferredComposerUploads?: boolean } };
    }
  ).__BUZZ_E2E__;
  const queueUntilSend =
    deferUploadsUntilSend &&
    (!e2eConfig || e2eConfig.mock?.deferredComposerUploads === true);
  const shouldQueueFile = React.useCallback(
    (file: File) => queueUntilSend && isVideoFile(file),
    [queueUntilSend],
  );
  const [uploadState, setUploadState] = React.useState<UploadState>({
    status: "idle",
  });
  /** Number of files currently in-flight. */
  const [uploadingCount, setUploadingCount] = React.useState(0);
  const [uploadingPreviews, setUploadingPreviews] = React.useState<
    UploadingAttachmentPreview[]
  >([]);
  const uploadingPreviewsRef = React.useRef(uploadingPreviews);
  uploadingPreviewsRef.current = uploadingPreviews;
  const [queuedAttachments, setQueuedAttachmentsState] = React.useState<
    QueuedMediaAttachment[]
  >([]);
  const queuedAttachmentsRef = React.useRef(queuedAttachments);
  queuedAttachmentsRef.current = queuedAttachments;
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const dispose = await listen<{
          id: string;
          sent: number;
          total: number;
        }>("media-upload-progress", (event) => {
          const { id, sent, total } = event.payload;
          if (total <= 0) return;
          const progress = Math.min(100, Math.round((sent / total) * 100));
          setUploadingPreviews((current) =>
            current.map((preview) =>
              uploadProgressId(preview.id) === id
                ? { ...preview, progress }
                : preview,
            ),
          );
        });
        if (cancelled) {
          dispose();
        } else {
          unlisten = dispose;
        }
      } catch {
        // Non-Tauri runtime (web dev, e2e mock) — no byte-level progress.
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
  const activeUploadingPreviewIdsRef = React.useRef(new Set<number>());
  const canceledUploadingPreviewIdsRef = React.useRef(new Set<number>());
  /**
   * Incremented whenever the composer's attachment set is replaced wholesale
   * (draft/channel switch, post-send clear, edit restore). Uploads capture the
   * epoch at start and discard their result if it no longer matches, so an
   * upload started against one draft can never land in another.
   */
  const uploadEpochRef = React.useRef(0);

  // ── Drag-over visual indicator state ───────────────────────────────
  const [isDragOver, setIsDragOver] = React.useState(false);
  /** Tracks nested dragenter/dragleave pairs so we only flip `isDragOver`
   *  when the pointer truly enters or leaves the drop target. */
  const dragDepthRef = React.useRef(0);
  /**
   * Internal slots array — may contain `null` for reserved-but-pending uploads.
   * Consumers see the filtered `pendingImeta` (nulls stripped) so the public
   * type stays `BlobDescriptor[]`.
   */
  const [imetaSlots, setImetaSlots] = React.useState<(BlobDescriptor | null)[]>(
    [],
  );

  const pendingImeta = React.useMemo(
    () => compactImetaSlots(imetaSlots),
    [imetaSlots],
  );

  const pendingImetaRef = React.useRef(pendingImeta);
  pendingImetaRef.current = pendingImeta;

  /**
   * Pre-edit originals of annotated attachments, keyed by the annotated
   * attachment's URL. Powers "revert to original" in the composer lightbox.
   * In-memory only, by design — cleared implicitly when the attachment
   * leaves the composer (send, remove, draft switch). Persisting revert
   * across a draft round-trip is an explicit non-goal (#1491 review).
   */
  const [originalsByUrl, setOriginalsByUrl] = React.useState<
    Map<string, BlobDescriptor>
  >(() => new Map());
  const originalsByUrlRef = React.useRef(originalsByUrl);
  originalsByUrlRef.current = originalsByUrl;

  /** Annotated URL → original URL (derived; handy for stable list keys). */
  const originalUrlByUrl = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const [url, original] of originalsByUrl) map.set(url, original.url);
    return map;
  }, [originalsByUrl]);

  // Prune originals whose annotated attachment is no longer pending —
  // covers remove, cancel, send-clear, and draft restore in one place.
  React.useEffect(() => {
    setOriginalsByUrl((prev) => {
      if (prev.size === 0) return prev;
      const liveUrls = new Set(pendingImeta.map((d) => d.url));
      let changed = false;
      const next = new Map<string, BlobDescriptor>();
      for (const [url, original] of prev) {
        if (liveUrls.has(url)) {
          next.set(url, original);
        } else {
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [pendingImeta]);

  /** Monotonic slot counter — ensures each batch gets unique indices even
   *  before React flushes the state update. */
  const nextSlotRef = React.useRef(0);
  const nextUploadingPreviewIdRef = React.useRef(0);
  const nextQueuedAttachmentIdRef = React.useRef(0);

  const updateQueuedVideoPoster = React.useCallback(
    (id: number, posterUrl: string) => {
      setQueuedAttachmentsState((current) =>
        current.map((attachment) =>
          attachment.id === id
            ? { ...attachment, previewUrl: posterUrl }
            : attachment,
        ),
      );
    },
    [],
  );

  const queueFiles = React.useCallback(
    (files: File[]) => {
      if (files.length === 0) return;

      const attachments = files.map((file) => {
        const id = nextQueuedAttachmentIdRef.current;
        nextQueuedAttachmentIdRef.current += 1;
        const previewUrl = file.type.startsWith("image/")
          ? URL.createObjectURL(file)
          : undefined;
        if (isVideoFile(file)) {
          void captureVideoPosterFrame(file).then((poster) => {
            if (poster) updateQueuedVideoPoster(id, poster.posterUrl);
          });
        }
        return { file, id, previewUrl, spoilered: false };
      });

      setQueuedAttachmentsState((current) => [...current, ...attachments]);
    },
    [updateQueuedVideoPoster],
  );

  const removeQueuedAttachment = React.useCallback((id: number) => {
    setQueuedAttachmentsState((current) => {
      const removed = current.find((attachment) => attachment.id === id);
      if (removed?.previewUrl?.startsWith("blob:")) {
        URL.revokeObjectURL(removed.previewUrl);
      }
      return current.filter((attachment) => attachment.id !== id);
    });
  }, []);

  const clearQueuedAttachments = React.useCallback(() => {
    setQueuedAttachmentsState((current) => {
      for (const attachment of current) {
        if (attachment.previewUrl?.startsWith("blob:")) {
          URL.revokeObjectURL(attachment.previewUrl);
        }
      }
      return [];
    });
  }, []);

  const restoreQueuedAttachments = React.useCallback(
    (attachments: QueuedMediaAttachment[]) => {
      clearQueuedAttachments();
      queueFiles(attachments.map((attachment) => attachment.file));
      setQueuedAttachmentsState((current) =>
        current.map((attachment, index) => ({
          ...attachment,
          spoilered: attachments[index]?.spoilered ?? false,
        })),
      );
    },
    [clearQueuedAttachments, queueFiles],
  );

  const toggleQueuedAttachmentSpoiler = React.useCallback((id: number) => {
    setQueuedAttachmentsState((current) =>
      current.map((attachment) =>
        attachment.id === id
          ? { ...attachment, spoilered: !attachment.spoilered }
          : attachment,
      ),
    );
  }, []);

  React.useEffect(
    () => () => {
      for (const attachment of queuedAttachmentsRef.current) {
        if (attachment.previewUrl?.startsWith("blob:")) {
          URL.revokeObjectURL(attachment.previewUrl);
        }
      }
    },
    [],
  );

  const isUploadCanceled = React.useCallback(
    (previewId?: number) =>
      previewId !== undefined &&
      canceledUploadingPreviewIdsRef.current.has(previewId),
    [],
  );

  const removeUploadingPreview = React.useCallback((id: number) => {
    setUploadingPreviews((prev) => prev.filter((preview) => preview.id !== id));
  }, []);

  const reserveUploadingPreview = React.useCallback(
    (file?: File, slotIndex?: number): number => {
      const id = nextUploadingPreviewIdRef.current;
      nextUploadingPreviewIdRef.current += 1;
      activeUploadingPreviewIdsRef.current.add(id);

      setUploadingPreviews((prev) => [
        ...prev,
        {
          id,
          filename: file?.name,
          slotIndex,
          // Normalize an extension-detected video to its MIME type so the
          // preview renders (and offers a spoiler toggle) like any other video.
          type: file ? (videoMimeForFile(file) ?? file.type) : undefined,
          uploadEpoch: uploadEpochRef.current,
        },
      ]);

      if (file && isVideoFile(file)) {
        void captureVideoPosterFrame(file).then((poster) => {
          if (!poster || isUploadCanceled(id)) return;
          setUploadingPreviews((prev) =>
            prev.map((preview) =>
              preview.id === id ? { ...preview, ...poster } : preview,
            ),
          );
        });
      }

      return id;
    },
    [isUploadCanceled],
  );

  const finishUpload = React.useCallback(
    (previewId?: number) => {
      if (previewId !== undefined) {
        if (!activeUploadingPreviewIdsRef.current.delete(previewId)) return;
        removeUploadingPreview(previewId);
      }
      setUploadingCount((c) => Math.max(0, c - 1));
    },
    [removeUploadingPreview],
  );

  /**
   * Mark a draft boundary: the composer's attachment set was replaced
   * wholesale (channel/draft switch, post-send clear, edit-target restore), so
   * every upload in flight belongs to a draft that is no longer on screen.
   *
   * Bumping the epoch makes those uploads discard their descriptors, and
   * retiring them drops their preview rows and their share of
   * `uploadingCount` immediately — otherwise the previous draft's uploads
   * would appear under the new draft and hold its send gate closed until they
   * finished. Marking them canceled also suppresses their completion and error
   * paths, so a failure the user has switched away from cannot raise a banner.
   *
   * Bump and retire are deliberately one operation: an epoch bump without the
   * retire is what left stale previews (and a stuck send gate) behind.
   */
  const beginNewDraftEpoch = React.useCallback(() => {
    uploadEpochRef.current += 1;
    const activeIds = activeUploadingPreviewIdsRef.current;
    if (activeIds.size === 0) return;
    // Snapshot before clearing the live set: the state updaters below run
    // lazily (and may be replayed), so they must not close over a set that
    // this callback empties before React invokes them — that would filter
    // against an empty set and subtract 0, leaving the stale preview and a
    // stuck send gate in the new draft.
    const retiredIds = new Set(activeIds);
    const retiredCount = retiredIds.size;
    activeIds.clear();
    for (const id of retiredIds) {
      canceledUploadingPreviewIdsRef.current.add(id);
    }
    setUploadingPreviews((prev) =>
      prev.filter((preview) => !retiredIds.has(preview.id)),
    );
    setUploadingCount((count) => Math.max(0, count - retiredCount));
  }, []);

  const cancelUpload = React.useCallback(
    (previewId: number) => {
      canceledUploadingPreviewIdsRef.current.add(previewId);
      const preview = uploadingPreviewsRef.current.find(
        (candidate) => candidate.id === previewId,
      );
      const slotIndex = preview?.slotIndex;
      // Only null the slot when the preview still belongs to the draft on
      // screen. A preview left over from a replaced draft carries that draft's
      // slotIndex, which may now address a different attachment.
      const isStalePreview =
        preview?.uploadEpoch !== undefined &&
        preview.uploadEpoch !== uploadEpochRef.current;
      if (slotIndex !== undefined && !isStalePreview) {
        setImetaSlots((prev) => {
          if (slotIndex >= prev.length) return prev;
          const next = [...prev];
          next[slotIndex] = null;
          return next;
        });
      }
      finishUpload(previewId);
    },
    [finishUpload],
  );

  /** Reserve `count` null slots at the end; returns the starting index. */
  const reserveSlots = React.useCallback((count: number): number => {
    const startIndex = nextSlotRef.current;
    nextSlotRef.current += count;
    setImetaSlots((prev) => {
      // Pad prev if needed (should already be the right length, but be safe)
      const padded =
        prev.length < startIndex
          ? [...prev, ...new Array<null>(startIndex - prev.length).fill(null)]
          : prev;
      return [...padded, ...new Array<null>(count).fill(null)];
    });
    return startIndex;
  }, []);

  /**
   * True when the composer's attachment set was replaced since `epoch` was
   * captured, meaning an upload that started then belongs to a draft that is
   * no longer on screen and must not write its descriptor.
   */
  const isUploadStale = React.useCallback(
    (epoch: number) => epoch !== uploadEpochRef.current,
    [],
  );

  /** Fill a previously-reserved slot by index. */
  const fillSlot = React.useCallback(
    (
      index: number,
      descriptor: BlobDescriptor,
      previewId?: number,
      epoch = uploadEpochRef.current,
    ) => {
      if (isUploadCanceled(previewId)) return;
      if (isUploadStale(epoch)) {
        finishUpload(previewId);
        return;
      }
      setImetaSlots((prev) => {
        const next = [...prev];
        next[index] = descriptor;
        return next;
      });
      finishUpload(previewId);
    },
    [finishUpload, isUploadCanceled, isUploadStale],
  );

  /** Append a single descriptor (no pre-reserved slot). */
  const onUploaded = React.useCallback(
    (
      descriptor: BlobDescriptor,
      previewId?: number,
      epoch = uploadEpochRef.current,
    ) => {
      if (isUploadCanceled(previewId)) return;
      if (isUploadStale(epoch)) {
        finishUpload(previewId);
        return;
      }
      nextSlotRef.current += 1;
      setImetaSlots((prev) => [...prev, descriptor]);
      finishUpload(previewId);
    },
    [finishUpload, isUploadCanceled, isUploadStale],
  );

  const onUploadError = React.useCallback(
    (err: unknown, previewId?: number) => {
      if (isUploadCanceled(previewId)) return;
      finishUpload(previewId);
      setUploadState({ status: "error", message: String(err) });
    },
    [finishUpload, isUploadCanceled],
  );

  const uploadFiles = React.useCallback(
    (files: File[]) => {
      if (files.length === 0) return;

      setUploadingCount((count) => count + files.length);
      const baseIndex = reserveSlots(files.length);
      // Pin the epoch at start: if the composer swaps drafts mid-flight, these
      // completions are discarded rather than written into the new draft.
      const epoch = uploadEpochRef.current;

      for (let index = 0; index < files.length; index++) {
        const file = files[index];
        const slotIndex = baseIndex + index;
        const previewId = reserveUploadingPreview(file, slotIndex);
        // Fire-and-forget each upload concurrently — slot preserves order.
        void (async () => {
          try {
            const descriptor = await uploadMediaFile(
              file,
              uploadProgressId(previewId),
            );
            fillSlot(slotIndex, descriptor, previewId, epoch);
          } catch (err) {
            onUploadError(err, previewId);
          }
        })();
      }
    },
    [fillSlot, onUploadError, reserveSlots, reserveUploadingPreview],
  );

  const openFilePicker = useFilePicker();

  const handlePaperclip = React.useCallback(async () => {
    if (queueUntilSend) {
      openFilePicker({ multiple: true }, (files) => {
        queueFiles(files.filter(shouldQueueFile));
        uploadFiles(files.filter((file) => !shouldQueueFile(file)));
      });
      return;
    }

    // Hold a single pending tick while the native picker is open + uploads
    // run in Rust. We don't know the file count until the dialog returns,
    // and uploads are already complete by then, so we just append each
    // descriptor when we get them back.
    const previewId = reserveUploadingPreview();
    setUploadingCount((c) => c + 1);
    const epoch = uploadEpochRef.current;
    try {
      const descriptors = await pickAndUploadMedia(uploadProgressId(previewId));
      if (isUploadCanceled(previewId)) return;
      finishUpload(previewId);
      if (isUploadStale(epoch)) return;
      for (const descriptor of descriptors) {
        nextSlotRef.current += 1;
        setImetaSlots((prev) => [...prev, descriptor]);
      }
    } catch (err) {
      if (isUploadCanceled(previewId)) return;
      onUploadError(err, previewId);
    }
  }, [
    queueUntilSend,
    finishUpload,
    isUploadCanceled,
    isUploadStale,
    onUploadError,
    openFilePicker,
    queueFiles,
    reserveUploadingPreview,
    shouldQueueFile,
    uploadFiles,
  ]);

  const handleDrop = React.useCallback(
    async (event: React.DragEvent<HTMLElement>) => {
      event.preventDefault();
      dragDepthRef.current = 0;
      setIsDragOver(false);
      const files = Array.from(event.dataTransfer.files);
      if (files.length === 0) return;

      // Accept any file. The Tauri layer and the relay enforce the deny-list
      // (active-content + executables) and size caps; everything else uploads.
      const validFiles = files;

      queueFiles(validFiles.filter(shouldQueueFile));
      uploadFiles(validFiles.filter((file) => !shouldQueueFile(file)));
    },
    [queueFiles, shouldQueueFile, uploadFiles],
  );

  const handleDragEnter = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      if (!isFileDrag(event)) return;
      event.preventDefault();
      dragDepthRef.current += 1;
      if (dragDepthRef.current === 1) {
        setIsDragOver(true);
      }
    },
    [],
  );

  const handleDragLeave = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      if (!isFileDrag(event)) return;
      event.preventDefault();
      dragDepthRef.current -= 1;
      if (dragDepthRef.current <= 0) {
        dragDepthRef.current = 0;
        setIsDragOver(false);
      }
    },
    [],
  );

  const handleDragOver = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      event.preventDefault();
    },
    [],
  );

  // Reset drag state when the drag operation ends outside the form (e.g. user
  // drops on another part of the window, presses Escape, or drags out of the
  // browser). Without this, `isDragOver` can stick if the browser doesn't fire
  // a balanced set of dragenter/dragleave events.
  React.useEffect(() => {
    function resetDragState() {
      dragDepthRef.current = 0;
      setIsDragOver(false);
    }
    window.addEventListener("drop", resetDragState);
    window.addEventListener("dragend", resetDragState);
    return () => {
      window.removeEventListener("drop", resetDragState);
      window.removeEventListener("dragend", resetDragState);
    };
  }, []);

  const handlePaste = React.useCallback(
    async (event: {
      clipboardData: DataTransfer;
      preventDefault: () => void;
    }) => {
      const items = Array.from(event.clipboardData.items);
      // Only clipboard items that are actual files — `getAsFile()` returns null
      // for text/string items, so pasting plain text never triggers an upload.
      const mediaFiles = items
        .filter((item) => item.kind === "file")
        .map((item) => item.getAsFile())
        .filter((f): f is File => f !== null);
      if (mediaFiles.length === 0) return;

      event.preventDefault();

      queueFiles(mediaFiles.filter(shouldQueueFile));
      uploadFiles(mediaFiles.filter((file) => !shouldQueueFile(file)));
    },
    [queueFiles, shouldQueueFile, uploadFiles],
  );

  /** Upload a File directly — used by Tiptap's editorProps.handlePaste. */
  const uploadFile = React.useCallback(
    async (file: File) => {
      if (shouldQueueFile(file)) {
        queueFiles([file]);
        return;
      }
      const previewId = reserveUploadingPreview(file);
      setUploadingCount((c) => c + 1);
      const epoch = uploadEpochRef.current;
      try {
        const descriptor = await uploadMediaFile(
          file,
          uploadProgressId(previewId),
        );
        onUploaded(descriptor, previewId, epoch);
      } catch (err) {
        onUploadError(err, previewId);
      }
    },
    [
      onUploaded,
      onUploadError,
      queueFiles,
      reserveUploadingPreview,
      shouldQueueFile,
    ],
  );

  /**
   * Upload an annotated replacement for an existing image attachment and
   * swap it into the same slot (attachment order is preserved). The pre-edit
   * descriptor is remembered in `originalsByUrl` so the edit can be reverted;
   * chained edits keep the earliest original as the single revert point.
   *
   * Returns the new descriptor, or null if `oldUrl` is no longer pending.
   * Rejects on upload failure (after surfacing the standard error banner) so
   * callers can keep their editing UI open.
   */
  const uploadEditedAttachment = React.useCallback(
    async (
      oldUrl: string,
      bytes: Uint8Array,
    ): Promise<BlobDescriptor | null> => {
      const oldDescriptor = pendingImetaRef.current.find(
        (d) => d.url === oldUrl,
      );
      if (!oldDescriptor) return null;

      // The annotated output is always PNG — swap the extension accordingly.
      const stem = (oldDescriptor.filename ?? "image").replace(/\.[^.]+$/, "");
      const filename = `${stem}.png`;

      const previewId = reserveUploadingPreview();
      setUploadingCount((c) => c + 1);
      try {
        const descriptor = await uploadMediaBytes(
          [...bytes],
          filename,
          uploadProgressId(previewId),
        );
        if (isUploadCanceled(previewId)) return null;
        finishUpload(previewId);
        setImetaSlots((prev) =>
          prev.map((d) => (d?.url === oldUrl ? descriptor : d)),
        );
        setOriginalsByUrl((prev) => {
          const next = new Map(prev);
          // Re-editing an annotated image keeps the earliest original.
          const original = prev.get(oldUrl) ?? oldDescriptor;
          next.delete(oldUrl);
          next.set(descriptor.url, original);
          return next;
        });
        return descriptor;
      } catch (err) {
        onUploadError(err, previewId);
        throw err;
      }
    },
    [finishUpload, isUploadCanceled, onUploadError, reserveUploadingPreview],
  );

  /**
   * Swap an annotated attachment back to its pre-edit original (same slot)
   * and forget the stored original. Returns the restored descriptor, or null
   * if the URL has no recorded original.
   */
  const revertAttachment = React.useCallback(
    (url: string): BlobDescriptor | null => {
      const original = originalsByUrlRef.current.get(url);
      if (!original) return null;
      setImetaSlots((prev) => prev.map((d) => (d?.url === url ? original : d)));
      setOriginalsByUrl((prev) => {
        const next = new Map(prev);
        next.delete(url);
        return next;
      });
      return original;
    },
    [],
  );

  const removeAttachment = React.useCallback((url: string) => {
    setImetaSlots((prev) => prev.map((d) => (d?.url === url ? null : d)));
  }, []);

  /** Public setter — replaces all slots (used by MessageComposer to clear/restore). */
  const setPendingImeta = React.useCallback(
    (action: React.SetStateAction<BlobDescriptor[]>) => {
      // A wholesale replacement means the composer's contents were swapped out
      // from under any in-flight upload: draft/channel switch, post-send clear,
      // or edit-target restore. Bump the epoch so those uploads discard their
      // results instead of landing in (or overwriting a slot reserved by) the
      // draft that is now on screen. The updater form is an append against the
      // *current* draft (e.g. agent-snapshot paste), so it must NOT bump.
      if (typeof action !== "function") {
        beginNewDraftEpoch();
        setImetaSlots(() => {
          nextSlotRef.current = action.length;
          return action;
        });
        return;
      }
      setImetaSlots((prev) => {
        const result = applyImetaUpdate(prev, action);
        nextSlotRef.current = result.length;
        return result;
      });
    },
    [beginNewDraftEpoch],
  );

  /**
   * True while any attachment upload is in flight.
   *
   * Send paths must gate on this: with `deferUploadsUntilSend`, only videos
   * are queued locally, so an in-flight photo/file is in neither
   * `pendingImeta` nor `queuedAttachments`. Sending mid-flight would publish
   * the message without that attachment and land the descriptor in an
   * already-cleared composer.
   */
  const isUploading = uploadingCount > 0;
  const queuedPreviews = React.useMemo<UploadingAttachmentPreview[]>(
    () =>
      queuedAttachments.map((attachment) => ({
        filename: attachment.file.name,
        id: attachment.id,
        posterUrl: attachment.previewUrl,
        spoilered: attachment.spoilered,
        type: videoMimeForFile(attachment.file) ?? attachment.file.type,
      })),
    [queuedAttachments],
  );

  return React.useMemo(
    () => ({
      cancelUpload,
      clearQueuedAttachments,
      handleDragEnter,
      handleDragLeave,
      handleDragOver,
      handleDrop,
      handlePaperclip,
      handlePaste,
      isDragOver,
      isUploading,
      originalUrlByUrl,
      pendingImeta,
      pendingImetaRef,
      queuedAttachments,
      queuedAttachmentsRef,
      queuedPreviews,
      removeAttachment,
      removeQueuedAttachment,
      restoreQueuedAttachments,
      revertAttachment,
      setPendingImeta,
      setUploadState,
      toggleQueuedAttachmentSpoiler,
      uploadEditedAttachment,
      uploadFile,
      uploadingCount,
      uploadingPreviews,
      uploadState,
    }),
    [
      cancelUpload,
      clearQueuedAttachments,
      handleDragEnter,
      handleDragLeave,
      handleDragOver,
      handleDrop,
      handlePaperclip,
      handlePaste,
      isDragOver,
      isUploading,
      originalUrlByUrl,
      pendingImeta,
      queuedAttachments,
      queuedPreviews,
      removeAttachment,
      removeQueuedAttachment,
      restoreQueuedAttachments,
      revertAttachment,
      setPendingImeta,
      toggleQueuedAttachmentSpoiler,
      uploadEditedAttachment,
      uploadFile,
      uploadingCount,
      uploadingPreviews,
      uploadState,
    ],
  );
}

export type MediaUploadController = ReturnType<typeof useMediaUpload>;
