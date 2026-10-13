// Web "read baseline" floor (fresh PWA login matches desktop).
//
// A fresh web install has no local read state, and the synced NIP-RS markers only
// cover contexts the desktop actually touched (channel keys + a bounded set of
// msg:/thread: keys, 7-day fetch horizon). The 14-day DM / thread-reply Inbox rows
// (#243) and older channel history therefore had no marker on web and all counted
// as unread (99+), while desktop showed 0.
//
// Fix: the first time read state finishes loading on web for a pubkey, persist a
// baseline = the latest synced marker (what the user had read up to on another
// device) or, with no markers at all, the first-login time. Every marker lookup is
// floored at that baseline, so anything older reads as "read"; anything newer still
// follows the real synced markers. Desktop is unaffected (no floor).

const BASELINE_KEY_PREFIX = "buzz.read-baseline.v1:";
// Guard against a skewed/future marker pushing the floor past "now".
const FUTURE_SLACK_SECONDS = 60;

export function readBaselineKey(pubkey: string): string {
  return `${BASELINE_KEY_PREFIX}${pubkey.toLowerCase()}`;
}

export function isWebPlatform(): boolean {
  try {
    return import.meta.env?.VITE_BUZZ_PLATFORM === "web";
  } catch {
    return false;
  }
}

export function loadReadBaseline(pubkey: string): number | null {
  try {
    const raw = localStorage.getItem(readBaselineKey(pubkey));
    if (!raw) return null;
    const value = Number(raw);
    return Number.isFinite(value) && value > 0 ? Math.floor(value) : null;
  } catch {
    return null;
  }
}

export function saveReadBaseline(pubkey: string, value: number): void {
  try {
    localStorage.setItem(readBaselineKey(pubkey), String(Math.floor(value)));
  } catch {
    /* quota: keep in memory only */
  }
}

/** Latest synced marker, else first-login time (now). */
export function computeReadBaseline(
  markerValues: Iterable<number>,
  nowSeconds: number,
): number {
  let latest = 0;
  for (const v of markerValues) {
    if (v > latest && v <= nowSeconds + FUTURE_SLACK_SECONDS) latest = v;
  }
  return latest > 0 ? Math.min(latest, nowSeconds) : nowSeconds;
}

export function applyReadBaseline(
  marker: number | null,
  baseline: number | null,
): number | null {
  if (baseline === null) return marker;
  return marker === null ? baseline : Math.max(marker, baseline);
}
