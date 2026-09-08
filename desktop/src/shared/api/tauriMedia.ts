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
  try {
    return await invokeTauriRaw<BlobDescriptor>(
      "upload_media_bytes_raw",
      bytes,
      {
        headers,
      },
    );
  } catch (error) {
    if (error instanceof Error) throw error;
    if (typeof error === "string" && error.trim()) throw new Error(error);
    if (
      typeof error === "object" &&
      error !== null &&
      "message" in error &&
      typeof error.message === "string" &&
      error.message.trim()
    ) {
      throw new Error(error.message);
    }
    throw new Error("Media upload failed.");
  }
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
  signal?: AbortSignal,
): Promise<Uint8Array<ArrayBuffer>> {
  if (signal?.aborted) {
    throw new DOMException("Media fetch cancelled", "AbortError");
  }

  const requestId = signal ? crypto.randomUUID() : undefined;
  // The Rust command replies with `tauri::ipc::Response`, so the bytes
  // arrive as a raw ArrayBuffer rather than a JSON number array.
  const request = invokeTauri<ArrayBuffer>("fetch_media_bytes", {
    requestId,
    url,
  });
  if (!signal || !requestId) return new Uint8Array(await request);

  let rejectCancellation: ((reason?: unknown) => void) | undefined;
  const cancellation = new Promise<never>((_resolve, reject) => {
    rejectCancellation = reject;
  });
  const onAbort = () => {
    void invokeTauri("cancel_media_fetch", { requestId })
      .catch(() => undefined)
      .finally(() => {
        rejectCancellation?.(
          new DOMException("Media fetch cancelled", "AbortError"),
        );
      });
  };
  signal.addEventListener("abort", onAbort, { once: true });

  try {
    const bytes = await Promise.race([request, cancellation]);
    return new Uint8Array(bytes);
  } finally {
    signal.removeEventListener("abort", onAbort);
    await invokeTauri("release_media_fetch", { requestId }).catch(
      () => undefined,
    );
  }
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
