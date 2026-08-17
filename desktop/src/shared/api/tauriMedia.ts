import { invoke as invokeTauriRaw, isTauri } from "@tauri-apps/api/core";
import { type BlobDescriptor, invokeTauri } from "./tauri";

function encodeRawIpcHeader(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return window
    .btoa(binary)
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "");
}

/** Transfer a browser File to Rust as a raw IPC body, avoiding JSON expansion. */
export async function uploadMediaFile(
  file: File,
  progressId?: string,
  signal?: AbortSignal,
  onDispatch?: () => void,
): Promise<BlobDescriptor> {
  const headers: Record<string, string> = {
    "x-buzz-filename": encodeRawIpcHeader(file.name),
  };
  if (progressId) {
    headers["x-buzz-progress-id"] = encodeRawIpcHeader(progressId);
  }

  if (signal?.aborted) throw new Error("upload cancelled");
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (signal?.aborted) throw new Error("upload cancelled");
  onDispatch?.();
  return invokeTauriRaw<BlobDescriptor>("upload_media_bytes_raw", bytes, {
    headers,
  });
}

/** Stop the native HTTP request associated with a background media upload. */
export async function cancelMediaUpload(progressId: string): Promise<void> {
  await invokeTauri("cancel_media_upload", { progressId });
}

/** Release the renderer's cancellation ownership after an upload settles. */
export async function releaseMediaUpload(progressId: string): Promise<void> {
  await invokeTauri("release_media_upload", { progressId });
}

/**
 * Open a native single-file picker constrained to images and upload the
 * chosen file. Non-image files are rejected in Rust (via MIME sniffing)
 * before the bytes leave the client, so discarded/non-image selections never
 * reach the relay. Resolves to `null` when the user cancels the dialog.
 */
export async function pickAndUploadImage(): Promise<BlobDescriptor | null> {
  return invokeTauri<BlobDescriptor | null>("pick_and_upload_image", {});
}

export async function uploadMedia(
  filePath: string,
  isTemp: boolean,
): Promise<BlobDescriptor> {
  return invokeTauri<BlobDescriptor>("upload_media", {
    filePath,
    isTemp,
  });
}

export async function pickAndUploadMedia(
  progressId?: string,
): Promise<BlobDescriptor[]> {
  return invokeTauri<BlobDescriptor[]>("pick_and_upload_media", { progressId });
}

export async function uploadMediaBytes(
  data: number[],
  filename?: string,
  /** Correlation id for `media-upload-progress` events from the Rust side. */
  progressId?: string,
): Promise<BlobDescriptor> {
  return invokeTauri<BlobDescriptor>("upload_media_bytes", {
    data,
    filename,
    progressId,
  });
}

/**
 * Fetch relay media bytes over IPC (Rust reqwest, VPN-tunneled).
 *
 * Used by the composer image editor: wrapping the bytes in a same-origin
 * `blob:` URL gives the canvas pixel access without CORS, so the media
 * proxy needs no special headers. The Rust side enforces the same URL
 * validation and size cap as the download commands.
 */
export async function fetchMediaBytes(
  url: string,
): Promise<Uint8Array<ArrayBuffer>> {
  // The Rust command replies with `tauri::ipc::Response`, so the bytes
  // arrive as a raw ArrayBuffer rather than a JSON number array.
  const bytes = await invokeTauri<ArrayBuffer>("fetch_media_bytes", { url });
  return new Uint8Array(bytes);
}

/**
 * Check whether a working LibreOffice install (`soffice`) is available on
 * this machine. Used to decide whether `.pptx` preview can go through the
 * high-fidelity `convertPptxToPdf` path instead of the client-side JS
 * renderer.
 *
 * The Rust side caches a *found* result for the process lifetime but
 * deliberately never caches a negative result, so calling this again after
 * the user installs LibreOffice (e.g. from a "Retry" button) picks up the
 * new install without an app restart.
 */
export async function checkLibreOfficeAvailable(): Promise<boolean> {
  return invokeTauri<boolean>("check_libreoffice_available", {});
}

/**
 * Convert `.pptx` bytes to PDF bytes via a local LibreOffice install, for
 * rendering through the existing `PdfPreview` component instead of the
 * lower-fidelity client-side `.pptx` renderer.
 *
 * Throws a human-readable error string if LibreOffice reported available but
 * failed on this specific file (corrupted install, unsupported file, etc.) —
 * callers should treat that the same as "LibreOffice not available" rather
 * than crashing the preview.
 */
export async function convertPptxToPdf(
  bytes: Uint8Array,
): Promise<Uint8Array<ArrayBuffer>> {
  const pdfBytes = await invokeTauri<ArrayBuffer>("convert_pptx_to_pdf", {
    bytes: Array.from(bytes),
  });
  return new Uint8Array(pdfBytes);
}

/** Read plain text without depending on embedded-webview clipboard grants. */
export async function readTextFromSystemClipboard(): Promise<string> {
  // E2E installs Tauri's mocked IPC surface in a browser page, where the SDK's
  // `isTauri()` marker remains false. Exercise the packaged-app command path in
  // that build so tests detect accidental regressions to permission-gated DOM
  // clipboard reads.
  if (isTauri() || import.meta.env.MODE === "e2e") {
    return invokeTauri<string>("read_clipboard_text");
  }

  const clipboard = navigator.clipboard;
  if (!clipboard?.readText) {
    throw new Error("Clipboard text reading is unavailable");
  }
  return clipboard.readText();
}

/** Write text through the native clipboard after an asynchronous workflow. */
export async function copyTextToSystemClipboard(
  text: string,
  html?: string,
): Promise<void> {
  await invokeTauri("copy_text_to_clipboard", { html, text });
}

/**
 * Fetch an agent snapshot attachment in memory, verifying size, SHA-256, and
 * snapshot decode before returning the bytes.
 *
 * Inputs come directly from the message's imeta fields; validation is
 * performed on the Rust side (same-relay URL, format-specific size cap,
 * hash + size integrity, and snapshot decode). Returns the raw bytes as a
 * number array so they can be passed to the existing preview/confirm APIs.
 *
 * Throws a human-readable error string on any validation failure.
 */
export async function fetchSnapshotBytes(args: {
  url: string;
  filename: string;
  expectedSha256: string;
  expectedSize: number;
}): Promise<number[]> {
  const buffer = await invokeTauri<ArrayBuffer>("fetch_snapshot_bytes", {
    url: args.url,
    filename: args.filename,
    expectedSha256: args.expectedSha256,
    expectedSize: args.expectedSize,
  });
  return Array.from(new Uint8Array(buffer));
}
