const MINUTE = 60;
const HOUR = 3600;
const DAY = 86400;

export function formatRelativeTime(unixSeconds: number): string {
  const now = Math.floor(Date.now() / 1000);
  const delta = Math.max(0, now - unixSeconds);

  if (delta < MINUTE) {
    return "just now";
  }

  if (delta < HOUR) {
    const minutes = Math.floor(delta / MINUTE);
    return `${minutes}m ago`;
  }

  if (delta < DAY) {
    const hours = Math.floor(delta / HOUR);
    return `${hours}h ago`;
  }

  const days = Math.floor(delta / DAY);
  return `${days}d ago`;
}
