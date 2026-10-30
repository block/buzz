import { uiT } from "@/shared/i18n/uiTranslation";
const MINUTE = 60;
const HOUR = 3600;
const DAY = 86400;

export function formatRelativeTime(unixSeconds: number): string {
  const now = Math.floor(Date.now() / 1000);
  const delta = Math.max(0, now - unixSeconds);

  if (delta < MINUTE) {
    return uiT("just now");
  }

  if (delta < HOUR) {
    const minutes = Math.floor(delta / MINUTE);
    return uiT("{0}m ago", { p0: minutes });
  }

  if (delta < DAY) {
    const hours = Math.floor(delta / HOUR);
    return uiT("{0}h ago", { p0: hours });
  }

  const days = Math.floor(delta / DAY);
  return uiT("{0}d ago", { p0: days });
}
