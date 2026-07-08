import { invokeTauri } from "./tauri";

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
