import { type BlobDescriptor, invokeTauri } from "./tauri";

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
 * Fetch relay media bytes over IPC (Rust reqwest, WARP-tunneled).
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
