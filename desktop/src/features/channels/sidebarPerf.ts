/**
 * Sidebar boot-paint measurement: marks the persisted-snapshot read and the
 * first fully-painted channel list per relay+identity. Split from hooks.ts to
 * keep that file under the per-file line cap; behavior unchanged.
 */

import type { inspectChannelSnapshot } from "@/features/channels/channelSnapshot";

export const CHANNELS_SNAPSHOT_DIAGNOSTIC_MARK =
  "buzz:sidebar:snapshot-diagnostic";
export const CHANNELS_FULL_SIDEBAR_PAINT_MARK =
  "buzz:sidebar:full-list-painted";
export const CHANNELS_BOOT_TO_FULL_SIDEBAR_MEASURE =
  "buzz:sidebar:boot-to-full-list-painted";

const markedSnapshotKeys = new Set<string>();
const measuredSidebarKeys = new Set<string>();
const scheduledSidebarKeys = new Set<string>();

function sidebarMeasurementKey(relayUrl: string, ownerPubkey: string): string {
  return `${relayUrl}\u0000${ownerPubkey.toLowerCase()}`;
}

export function markSnapshotDiagnostic(
  relayUrl: string,
  ownerPubkey: string,
  diagnostics: ReturnType<typeof inspectChannelSnapshot>["diagnostics"],
): void {
  if (typeof performance === "undefined") return;
  const key = sidebarMeasurementKey(relayUrl, ownerPubkey);
  if (markedSnapshotKeys.has(key)) return;
  markedSnapshotKeys.add(key);
  performance.mark(CHANNELS_SNAPSHOT_DIAGNOSTIC_MARK, {
    detail: { ...diagnostics, relayUrl },
  });
  console.info("[sidebar-perf] snapshot", { ...diagnostics, relayUrl });
}

export function measureFullSidebarPaint(
  relayUrl: string,
  ownerPubkey: string,
  channelCount: number,
): void {
  if (typeof performance === "undefined") return;
  const key = sidebarMeasurementKey(relayUrl, ownerPubkey);
  if (measuredSidebarKeys.has(key) || scheduledSidebarKeys.has(key)) return;
  scheduledSidebarKeys.add(key);

  // The channels have committed to the shared query cache; two animation frames
  // put the mark after React's sidebar DOM commit and the browser's next paint.
  window.requestAnimationFrame(() => {
    window.requestAnimationFrame(() => {
      scheduledSidebarKeys.delete(key);
      if (measuredSidebarKeys.has(key)) return;
      measuredSidebarKeys.add(key);
      performance.mark(CHANNELS_FULL_SIDEBAR_PAINT_MARK, {
        detail: { channelCount, relayUrl },
      });
      performance.measure(CHANNELS_BOOT_TO_FULL_SIDEBAR_MEASURE, {
        detail: { channelCount, relayUrl },
        duration: performance.now(),
        start: 0,
      });
      const measure = performance
        .getEntriesByName(CHANNELS_BOOT_TO_FULL_SIDEBAR_MEASURE)
        .at(-1);
      console.info("[sidebar-perf] full list painted", {
        channelCount,
        durationMs: measure?.duration,
        relayUrl,
      });
    });
  });
}
