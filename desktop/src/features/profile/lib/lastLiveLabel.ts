/**
 * Human-readable recency label for an activity surface's "Last live" pill.
 * `null` means the agent has never produced observable activity.
 */
export function formatLastLiveLabel(
  timestamp: number | null,
  now: number,
): string {
  if (timestamp === null) {
    return "No activity yet";
  }

  const elapsedMs = Math.max(0, now - timestamp);
  const totalSeconds = Math.floor(elapsedMs / 1000);
  if (totalSeconds < 60) {
    return "Just now";
  }

  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return `${totalMinutes}m ago`;
  }

  const totalHours = Math.floor(totalMinutes / 60);
  if (totalHours < 24) {
    return `${totalHours}h ago`;
  }

  const totalDays = Math.floor(totalHours / 24);
  if (totalDays < 7) {
    return `${totalDays}d ago`;
  }

  const totalWeeks = Math.floor(totalDays / 7);
  return `${totalWeeks}w ago`;
}
