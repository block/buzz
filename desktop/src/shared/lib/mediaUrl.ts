/**
 * Rewrite relay media URLs to use the localhost streaming proxy.
 *
 * WKWebView's networking stack bypasses WARP, so direct <img src> requests
 * to the relay get 403'd by Cloudflare Access. The localhost proxy routes
 * fetches through the Rust backend (via reqwest), which goes through WARP.
 *
 * For video, the proxy streams via axum — no buffering the entire file.
 * Images and other media also benefit from this path.
 *
 * Only URLs hosted on the Buzz relay are rewritten. External Blossom URLs
 * (e.g. nostr.build, void.cat) are returned unchanged — they aren't behind
 * Cloudflare Access and can be loaded directly by WKWebView. Without this
 * origin check, external Blossom URLs would be proxied to the wrong server
 * (the Buzz relay), resulting in 404s.
 */

import { invoke } from "@tauri-apps/api/core";

// Matches: https://anything.com/media/{64-hex}.{ext}
// Also matches thumbnails: /media/{64-hex}.thumb.jpg
const RELAY_MEDIA_RE =
  /^(?:https?:\/\/[^/]+)\/media\/([\da-f]{64}(?:\.thumb)?\.(?:jpg|png|gif|webp|mp4)(?:\?.*)?)$/;

/** Cached proxy port — fetched once from the Tauri backend. */
let cachedPort: number | null = null;
let portPromise: Promise<number | null> | null = null;

/** Cached relay origin (e.g. "https://buzz-oss.stage.blox.sqprod.co"). */
let cachedRelayOrigin: string | null = null;

const POLL_INTERVAL_MS = 100;
const POLL_TIMEOUT_MS = 5000;

/**
 * Poll `get_media_proxy_port` until we get a non-zero port or timeout.
 * Also fetches the relay HTTP base URL for origin-checking.
 * Returns the port, or null if the proxy never came up.
 */
async function fetchProxyPort(): Promise<number | null> {
  // Fetch relay origin in parallel — fire-and-forget, no retry needed.
  if (!cachedRelayOrigin) {
    invoke<string>("get_relay_http_url")
      .then((url) => {
        cachedRelayOrigin = url.replace(/\/+$/, "");
      })
      .catch(() => {});
  }

  const deadline = Date.now() + POLL_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const port = await invoke<number>("get_media_proxy_port");
      if (port > 0) {
        cachedPort = port;
        return port;
      }
    } catch {
      // invoke failed (e.g. Tauri IPC not ready yet) — keep retrying
    }
    await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
  }
  return null;
}

/** Eagerly fetch the port at module load so it's ready by first render. */
// The try/catch inside fetchProxyPort handles non-Tauri environments gracefully
// (invoke will throw, we retry until timeout, then give up — no side effects).
if (typeof window !== "undefined") {
  portPromise = fetchProxyPort();
}

/**
 * Reset module-level caches so the next render re-fetches the proxy port
 * and relay origin for the new workspace.
 */
export function resetMediaCaches(): void {
  cachedPort = null;
  portPromise = null;
  cachedRelayOrigin = null;
}

/**
 * Build the local proxy URL with an IPv4 literal. The Rust proxy binds
 * `127.0.0.1:0`, not `::1`, and some WebViews resolve `localhost` to IPv6
 * first. Matching the bind address avoids machine-dependent image failures.
 */
export function mediaProxyUrl(port: number, mediaPath: string): string {
  return `http://127.0.0.1:${port}/media/${mediaPath}`;
}

/**
 * If `url` is a Blossom media URL hosted on the Buzz relay, rewrite it
 * to go through the local streaming proxy. External Blossom URLs and
 * non-Blossom URLs are returned unchanged.
 *
 * Falls back to buzz-media:// if the proxy port isn't available yet.
 */
export function rewriteRelayUrl(url: string): string {
  const m = RELAY_MEDIA_RE.exec(url);
  if (!m) return url;

  // Only proxy URLs that belong to our relay. External Blossom URLs
  // (different origin) pass through unchanged — they work fine via WKWebView.
  // If the relay origin isn't cached yet, fall through to the rewrite path
  // as a safe default (relay URLs need the proxy to avoid Cloudflare 403s).
  if (cachedRelayOrigin && !url.startsWith(`${cachedRelayOrigin}/`)) {
    return url;
  }

  if (cachedPort && cachedPort > 0) {
    return mediaProxyUrl(cachedPort, m[1]);
  }

  if (!portPromise && typeof window !== "undefined") {
    portPromise = fetchProxyPort();
  }

  return `buzz-media://localhost/media/${m[1]}`;
}
