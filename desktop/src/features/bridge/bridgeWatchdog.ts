/** No heartbeat for this long while the page is visible: resubscribe and re-fetch the bridge heads. */
export const BRIDGE_WATCHDOG_STALE_S = 45;
/** At most one forced resubscribe per this many seconds. */
export const BRIDGE_WATCHDOG_MIN_GAP_S = 30;

/** Pure decision for the watchdog (unit-tested): resubscribe when visible, stale (or never heard / key failed) and not just kicked. */
export function shouldKickBridge(o: { visible: boolean; lastHeardTs: number | null; keyFailed: boolean; now: number; lastKick: number }): boolean {
  if (!o.visible) return false;
  if (o.now - o.lastKick < BRIDGE_WATCHDOG_MIN_GAP_S) return false;
  if (o.keyFailed) return true;
  return o.lastHeardTs === null || o.now - o.lastHeardTs > BRIDGE_WATCHDOG_STALE_S;
}
