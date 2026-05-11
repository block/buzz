import * as React from "react";

import {
  type BlobDescriptor,
  pickAndUploadMedia,
  uploadMediaBytes,
} from "@/shared/api/tauri";

export const ALLOWED_MEDIA_TYPES = [
  "image/jpeg",
  "image/png",
  "image/gif",
  "image/webp",
  "video/mp4",
  "video/quicktime",
  "video/x-matroska",
  "video/webm",
  "video/x-msvideo",
];

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

/** True when the drag payload contains files (not plain text or URLs). */
function isFileDrag(event: React.DragEvent): boolean {
  return event.dataTransfer?.types.includes("Files") ?? false;
}

export function useMediaUpload() {
  const [uploadState, setUploadState] = React.useState<UploadState>({
    status: "idle",
  });
  /** Number of files currently in-flight. */
  const [uploadingCount, setUploadingCount] = React.useState(0);

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

  /** Monotonic slot counter — ensures each batch gets unique indices even
   *  before React flushes the state update. */
  const nextSlotRef = React.useRef(0);

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

  /** Fill a previously-reserved slot by index. */
  const fillSlot = React.useCallback(
    (index: number, descriptor: BlobDescriptor) => {
      setImetaSlots((prev) => {
        const next = [...prev];
        next[index] = descriptor;
        return next;
      });
      setUploadingCount((c) => Math.max(0, c - 1));
    },
    [],
  );

  /** Append a single descriptor (no pre-reserved slot). */
  const onUploaded = React.useCallback((descriptor: BlobDescriptor) => {
    nextSlotRef.current += 1;
    setImetaSlots((prev) => [...prev, descriptor]);
    setUploadingCount((c) => Math.max(0, c - 1));
  }, []);

  const onUploadError = React.useCallback((err: unknown) => {
    setUploadingCount((c) => Math.max(0, c - 1));
    setUploadState({ status: "error", message: String(err) });
  }, []);

  const handlePaperclip = React.useCallback(async () => {
    setUploadingCount((c) => c + 1);
    try {
      const descriptor = await pickAndUploadMedia();
      if (descriptor) {
        onUploaded(descriptor);
      } else {
        setUploadingCount((c) => Math.max(0, c - 1));
      }
    } catch (err) {
      onUploadError(err);
    }
  }, [onUploaded, onUploadError]);

  const handleDrop = React.useCallback(
    async (event: React.DragEvent<HTMLFormElement>) => {
      event.preventDefault();
      dragDepthRef.current = 0;
      setIsDragOver(false);
      const files = Array.from(event.dataTransfer.files);
      if (files.length === 0) return;

      const validFiles = files.filter((f) =>
        ALLOWED_MEDIA_TYPES.includes(f.type),
      );

      if (validFiles.length === 0) {
        setUploadState({
          status: "error",
          message:
            "Unsupported file type. Supported: JPEG, PNG, GIF, WebP, MP4, MOV, MKV, WebM, AVI",
        });
        return;
      }

      setUploadingCount((c) => c + validFiles.length);
      const baseIndex = reserveSlots(validFiles.length);

      for (let i = 0; i < validFiles.length; i++) {
        const file = validFiles[i];
        const slotIndex = baseIndex + i;
        // Fire-and-forget each upload concurrently — slot preserves order
        (async () => {
          try {
            const buffer = await file.arrayBuffer();
            const descriptor = await uploadMediaBytes([
              ...new Uint8Array(buffer),
            ]);
            fillSlot(slotIndex, descriptor);
          } catch (err) {
            onUploadError(err);
          }
        })();
      }
    },
    [reserveSlots, fillSlot, onUploadError],
  );

  const handleDragEnter = React.useCallback(
    (event: React.DragEvent<HTMLFormElement>) => {
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
    (event: React.DragEvent<HTMLFormElement>) => {
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
    (event: React.DragEvent<HTMLFormElement>) => {
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
      const mediaFiles = items
        .filter((item) => ALLOWED_MEDIA_TYPES.includes(item.type))
        .map((item) => item.getAsFile())
        .filter((f): f is File => f !== null);
      if (mediaFiles.length === 0) return;

      event.preventDefault();

      setUploadingCount((c) => c + mediaFiles.length);
      const baseIndex = reserveSlots(mediaFiles.length);

      for (let i = 0; i < mediaFiles.length; i++) {
        const file = mediaFiles[i];
        const slotIndex = baseIndex + i;
        (async () => {
          try {
            const buffer = await file.arrayBuffer();
            const descriptor = await uploadMediaBytes([
              ...new Uint8Array(buffer),
            ]);
            fillSlot(slotIndex, descriptor);
          } catch (err) {
            onUploadError(err);
          }
        })();
      }
    },
    [reserveSlots, fillSlot, onUploadError],
  );

  /** Upload a File directly — used by Tiptap's editorProps.handlePaste. */
  const uploadFile = React.useCallback(
    async (file: File) => {
      if (!ALLOWED_MEDIA_TYPES.includes(file.type)) return;
      setUploadingCount((c) => c + 1);
      try {
        const buffer = await file.arrayBuffer();
        const descriptor = await uploadMediaBytes([...new Uint8Array(buffer)]);
        onUploaded(descriptor);
      } catch (err) {
        onUploadError(err);
      }
    },
    [onUploaded, onUploadError],
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

  const isUploading = uploadingCount > 0;

  return {
    handleDragEnter,
    handleDragLeave,
    handleDragOver,
    handleDrop,
    handlePaperclip,
    handlePaste,
    isDragOver,
    isUploading,
    pendingImeta,
    pendingImetaRef,
    removeAttachment,
    setPendingImeta,
    setUploadState,
    uploadFile,
    uploadingCount,
    uploadState,
  };
}
