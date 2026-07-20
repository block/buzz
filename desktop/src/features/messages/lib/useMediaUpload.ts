import * as React from "react";

import {
  type BlobDescriptor,
  pickAndUploadMedia,
  uploadMediaBytes,
} from "@/shared/api/tauri";

import { captureVideoPosterFrame } from "./videoPoster";
import {
  UPLOAD_JOB_ID_BASE,
  cancelUploadJob,
  getUploadJobsVersion,
  startChannelUpload,
  subscribeUploadJobs,
  takeChannelCompletions,
  takeChannelErrors,
  useChannelUploadJobs,
} from "./uploadJobsStore";

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
  type?: string;
};

/** Correlation id for the Rust `media-upload-progress` events. */
function uploadProgressId(previewId: number): string {
  return `composer-upload-${previewId}`;
}

/** True when the drag payload contains files (not plain text or URLs). */
function isFileDrag(event: React.DragEvent<HTMLElement>): boolean {
  return event.dataTransfer?.types.includes("Files") ?? false;
}

/**
 * @param channelId The channel this composer is attached to. Background upload
 *   jobs are keyed by it so in-flight uploads stay with the entry field where
 *   they began, survive navigating away and back, and never follow you to
 *   another channel. Falls back to a shared key when absent.
 */
export function useMediaUpload(channelId?: string | null) {
  const channelKey = channelId ?? "__composer__";
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
    () => imetaSlots.filter((d): d is BlobDescriptor => d !== null),
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

  // ── Background upload jobs (drag/drop, paste, editor paste) ──────────────
  // These run in the workspace-scoped store, keyed by channel, so they survive
  // navigation and feed the global indicator. This composer renders only its
  // own channel's jobs and drains their finished descriptors into `imetaSlots`.
  const storeJobs = useChannelUploadJobs(channelId ?? null);
  const jobsVersion = React.useSyncExternalStore(
    subscribeUploadJobs,
    getUploadJobsVersion,
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: jobsVersion is the reactive trigger — it re-runs the drain whenever the store changes (mount, or a job finishing while this channel is open).
  React.useEffect(() => {
    const done = takeChannelCompletions(channelKey);
    if (done.length > 0) {
      nextSlotRef.current += done.length;
      setImetaSlots((prev) => [...prev, ...done]);
    }
    const errors = takeChannelErrors(channelKey);
    if (errors.length > 0) {
      setUploadState({ status: "error", message: errors[errors.length - 1] });
    }
  }, [channelKey, jobsVersion]);

  const storePreviews = React.useMemo<UploadingAttachmentPreview[]>(
    () =>
      storeJobs.map((job) => ({
        id: job.id,
        filename: job.filename,
        type: job.mediaType,
        posterUrl: job.posterUrl,
        progress: job.pct,
      })),
    [storeJobs],
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
        { id, filename: file?.name, slotIndex, type: file?.type },
      ]);

      if (file?.type.startsWith("video/")) {
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

  const cancelUpload = React.useCallback(
    (previewId: number) => {
      // Store-driven jobs live in a high id range — hand them to the store,
      // which aborts staging, deletes the temp file, and discards any result.
      if (previewId >= UPLOAD_JOB_ID_BASE) {
        cancelUploadJob(previewId);
        return;
      }
      canceledUploadingPreviewIdsRef.current.add(previewId);
      const slotIndex = uploadingPreviewsRef.current.find(
        (preview) => preview.id === previewId,
      )?.slotIndex;
      if (slotIndex !== undefined) {
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

  const onUploadError = React.useCallback(
    (err: unknown, previewId?: number) => {
      if (isUploadCanceled(previewId)) return;
      finishUpload(previewId);
      setUploadState({ status: "error", message: String(err) });
    },
    [finishUpload, isUploadCanceled],
  );

  const handlePaperclip = React.useCallback(async () => {
    // Hold a single pending tick while the native picker is open + uploads
    // run in Rust. We don't know the file count until the dialog returns,
    // and uploads are already complete by then, so we just append each
    // descriptor when we get them back.
    const previewId = reserveUploadingPreview();
    setUploadingCount((c) => c + 1);
    try {
      const descriptors = await pickAndUploadMedia();
      if (isUploadCanceled(previewId)) return;
      finishUpload(previewId);
      for (const descriptor of descriptors) {
        nextSlotRef.current += 1;
        setImetaSlots((prev) => [...prev, descriptor]);
      }
    } catch (err) {
      if (isUploadCanceled(previewId)) return;
      onUploadError(err, previewId);
    }
  }, [finishUpload, isUploadCanceled, onUploadError, reserveUploadingPreview]);

  const handleDrop = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      event.preventDefault();
      dragDepthRef.current = 0;
      setIsDragOver(false);
      const files = Array.from(event.dataTransfer.files);
      if (files.length === 0) return;

      // Accept any file. The Tauri layer and the relay enforce the deny-list
      // (active-content + executables) and size caps; everything else uploads.
      // Each upload becomes a background job attached to this channel — it runs
      // independent of this component's lifetime and its descriptor is drained
      // back into `imetaSlots` on completion.
      for (const file of files) {
        startChannelUpload(channelKey, file);
      }
    },
    [channelKey],
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

      for (const file of mediaFiles) {
        startChannelUpload(channelKey, file);
      }
    },
    [channelKey],
  );

  /** Upload a File directly — used by Tiptap's editorProps.handlePaste. */
  const uploadFile = React.useCallback(
    (file: File) => {
      startChannelUpload(channelKey, file);
    },
    [channelKey],
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
      setImetaSlots((prev) => {
        const current = prev.filter((d): d is BlobDescriptor => d !== null);
        const next = typeof action === "function" ? action(current) : action;
        nextSlotRef.current = next.length;
        return next;
      });
    },
    [],
  );

  // Local previews (annotation/paperclip) + background store jobs for this
  // channel. Store jobs carry ids ≥ UPLOAD_JOB_ID_BASE so they never collide.
  const allUploadingPreviews = React.useMemo(
    () => [...uploadingPreviews, ...storePreviews],
    [uploadingPreviews, storePreviews],
  );
  const totalUploadingCount = uploadingCount + storeJobs.length;
  const isUploading = totalUploadingCount > 0;

  return React.useMemo(
    () => ({
      cancelUpload,
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
      removeAttachment,
      revertAttachment,
      setPendingImeta,
      setUploadState,
      uploadEditedAttachment,
      uploadFile,
      uploadingCount: totalUploadingCount,
      uploadingPreviews: allUploadingPreviews,
      uploadState,
    }),
    [
      cancelUpload,
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
      removeAttachment,
      revertAttachment,
      setPendingImeta,
      uploadEditedAttachment,
      uploadFile,
      totalUploadingCount,
      allUploadingPreviews,
      uploadState,
    ],
  );
}

export type MediaUploadController = ReturnType<typeof useMediaUpload>;
