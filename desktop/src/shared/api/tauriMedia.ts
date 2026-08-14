import { invoke as invokeTauriRaw, isTauri } from "@tauri-apps/api/core";
import { type BlobDescriptor, invokeTauri } from "./tauri";
import { withVolatileWork } from "@/shared/lib/volatileWorkRegistry";

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

  return invokeTauriRaw<BlobDescriptor>("upload_media_bytes_raw", bytes, {
    headers,
  });
}

/** Stop the native HTTP request associated with a background media upload. */
export async function cancelMediaUpload(progressId: string): Promise<void> {
  await invokeTauri("cancel_media_upload", { progressId });
}

/**
 * Open a native single-file picker constrained to images and upload the
 * chosen file. Non-image files are rejected in Rust (via MIME sniffing)
 * before the bytes leave the client, so discarded/non-image selections never
 * reach the relay. Resolves to `null` when the user cancels the dialog.
 */
export async function pickAndUploadImage(): Promise<BlobDescriptor | null> {
  // Same reload-destroyed window as `pickAndUploadMedia`: an open OS dialog
  // then a Rust-side upload. Hold the volatile-work registry across it (release
  // on resolve or cancel) so the idle backstop withholds the reload.
  return withVolatileWork("pick-and-upload-image", () =>
    invokeTauri<BlobDescriptor | null>("pick_and_upload_image", {}),
  );
}

/**
 * Open the native multi-file picker and upload the chosen files, reporting
 * per-file progress under `progressId`. Rust performs the upload; this resolves
 * once every selected file has landed.
 */
export async function pickAndUploadMedia(
  progressId?: string,
): Promise<BlobDescriptor[]> {
  // The native picker returns immediately from JS but holds an open OS dialog
  // and then a Rust-side upload — reload-destroyed work that no other signal
  // (`uploadingCount`, mutations) covers for every caller. Hold the app-global
  // volatile-work registry across the whole operation (release on resolve or
  // reject) so the idle backstop withholds the reload while the picker is open.
  return withVolatileWork("pick-and-upload-media", () =>
    invokeTauri<BlobDescriptor[]>("pick_and_upload_media", { progressId }),
  );
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
