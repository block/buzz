// Relay URL resolution for the web build.
// Default comes from build-time VITE_BUZZ_RELAY_URL (prod: wss://buzz.huladesk.com, dev: ws://localhost:3000).
// The active community's relay (set by apply_workspace) wins at runtime. A `?relay=` override is DEV-only.
const DEFAULT =
  (import.meta.env.VITE_BUZZ_RELAY_URL as string | undefined) ||
  "ws://localhost:3000";
let applied: string | null = null;

export function setAppliedRelay(url: string | null) {
  applied = url;
}
const OVERRIDE_KEY = "buzz-web-relay-override";
const HULA_RELAY = "wss://buzz.huladesk.com";
const isLocalHost = (h: string) => ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"].includes(h);
export function defaultRelayWsUrl(): string {
  if (
    import.meta.env.DEV ||
    import.meta.env.VITE_BUZZ_ALLOW_RELAY_OVERRIDE === "1"
  ) {
    // `?relay=` is remembered: returning from a signer app (Primal on iOS reopens the bare origin) or launching the home-screen
    // PWA drops the query, and without it a fresh session fell back to the local dev default and showed "join a community".
    const q = new URLSearchParams(location.search).get("relay");
    try {
      if (q) localStorage.setItem(OVERRIDE_KEY, q);
      const saved = localStorage.getItem(OVERRIDE_KEY);
      if (saved) return saved;
    } catch {
      /* storage denied */
    }
    if (q) return q;
  }
  // Dev server opened from another device (tailnet/LAN): its localhost relay is unreachable there, use the Hula relay.
  try {
    if (new URL(DEFAULT).hostname !== "" && isLocalHost(new URL(DEFAULT).hostname) && !isLocalHost(location.hostname)) return HULA_RELAY;
  } catch {
    /* bad URL */
  }
  return DEFAULT;
}
export const relayWsUrl = (): string => applied ?? defaultRelayWsUrl();
export const wsToHttp = (u: string): string =>
  u
    .trim()
    .replace(/\/+$/, "")
    .replace(/^wss:/, "https:")
    .replace(/^ws:/, "http:");
export const relayHttpBase = (): string => wsToHttp(relayWsUrl());
