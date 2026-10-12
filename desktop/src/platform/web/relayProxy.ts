// Where the web build sends relay HTTP requests (/query, /events, /upload, /media/...).
//
// - Dev server: cross-origin relays go through Vite's same-origin `/__relay/<encoded base>` proxy (vite.config.web.ts).
// - Production static hosting (S3/CloudFront): there is no `/__relay` server; a relative URL would be answered by the
//   SPA fallback with index.html ("Unexpected token '<'"). So production talks to the relay origin directly (the relay
//   must allow our origin via CORS), or — if VITE_BUZZ_RELAY_PROXY_PREFIX is set at build time (e.g. "/relay") — through
//   a same-origin CDN path that forwards to the default relay.

const DEV = import.meta.env?.DEV ?? false;
const PROXY_PREFIX = ((import.meta.env?.VITE_BUZZ_RELAY_PROXY_PREFIX as string | undefined) ?? "").replace(/\/+$/, "");
const DEFAULT_RELAY_HTTP = ((import.meta.env?.VITE_BUZZ_RELAY_URL as string | undefined) ?? "")
  .replace(/^ws(s?):/, "http$1:");

function sameOrigin(a: string, b: string): boolean {
  try {
    return new URL(a).origin === new URL(b).origin;
  } catch {
    return false;
  }
}

export type RelayRouteOpts = { dev?: boolean; prefix?: string; defaultRelayHttp?: string; origin?: string };

/** URL to fetch for `path` on relay HTTP base `base` (an absolute http(s) origin). */
export function relayFetchUrl(base: string, path: string, opts: RelayRouteOpts = {}): string {
  const dev = opts.dev ?? DEV;
  const prefix = opts.prefix ?? PROXY_PREFIX;
  const def = opts.defaultRelayHttp ?? DEFAULT_RELAY_HTTP;
  const origin = opts.origin ?? location.origin;
  const b = base.replace(/\/+$/, "");
  if (sameOrigin(b, origin)) return `${b}${path}`;
  if (dev) return `/__relay/${encodeURIComponent(b)}${path}`;
  if (prefix && def && sameOrigin(b, def)) return `${prefix}${path}`;
  return `${b}${path}`;
}
