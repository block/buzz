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
  const bytes = await invokeTauri<number[]>("fetch_media_bytes", { url });
  return new Uint8Array(bytes);
}
