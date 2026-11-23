/** Preserve wire timestamps and compare seconds and milliseconds at the same scale. */
export function sidebarFlagTimeMillis(timestamp: number): number {
  return timestamp >= 1_000_000_000 && timestamp < 10_000_000_000
    ? timestamp * 1_000
    : timestamp;
}

export function nextSidebarFlagTime(previous: number | undefined): number {
  return Math.max(Date.now(), sidebarFlagTimeMillis(previous ?? 0) + 1);
}
