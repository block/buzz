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
export function defaultRelayWsUrl(): string {
  if (
    import.meta.env.DEV ||
    import.meta.env.VITE_BUZZ_ALLOW_RELAY_OVERRIDE === "1"
  ) {
    const q = new URLSearchParams(location.search).get("relay");
    if (q) return q;
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
