/**
 * Pure helpers for the "Schedule for Later" composer feature.
 *
 * Time plumbing stays in one place so the dialog, the delivery loop, and the
 * Scheduled view agree on the same conversions:
 *
 *   datetime-local input  →  RFC 3339 / ISO 8601  →  Unix seconds (queue)
 *
 * The native `<input type="datetime-local">` is timezone-naive: its value is
 * interpreted as *local* wall-clock time, and `new Date(value)` reinterprets
 * it in the local timezone, so the round trip is exact without any offset
 * math.
 */

export type SchedulePreset = {
  label: string;
  deltaMs: number;
};

export const SCHEDULE_PRESETS: SchedulePreset[] = [
  { label: "In 1 hour", deltaMs: 60 * 60 * 1000 },
  { label: "In 3 hours", deltaMs: 3 * 60 * 60 * 1000 },
  { label: "Tomorrow 9 AM", deltaMs: nextLocal9amDelta() },
];

/** Minutes from `now` until the next local 9:00 AM (tomorrow if after 9am). */
function nextLocal9amDelta(): number {
  const now = new Date();
  const nineAm = new Date(now);
  nineAm.setHours(9, 0, 0, 0);
  if (nineAm.getTime() <= now.getTime()) {
    nineAm.setDate(nineAm.getDate() + 1);
  }
  return nineAm.getTime() - now.getTime();
}

/** Default starting value for the datetime picker: one hour from now. */
export function defaultScheduleDatetime(): string {
  return unixToDatetimeLocal(Math.floor(Date.now() / 1000) + 60 * 60);
}

/**
 * Convert a `datetime-local` input value (local wall-clock, `YYYY-MM-DDTHH:mm`)
 * into an RFC 3339 timestamp. Returns `null` when the value is unparseable or
 * empty, which lets the dialog fail closed without guessing.
 */
export function datetimeLocalToIso(value: string): string | null {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value)) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return date.toISOString();
}

/**
 * Convert a Unix timestamp (seconds) into a `datetime-local` input value in
 * the local timezone. Used to pre-fill the picker when rescheduling.
 */
export function unixToDatetimeLocal(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Validate a Unix delivery timestamp is in the future. */
export function isFutureTimestamp(timestamp: number): boolean {
  return timestamp > Math.floor(Date.now() / 1000);
}

const DELIVERY_TIME_FORMATTER = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/** Local-timezone, human-readable delivery time for list rows and toasts. */
export function formatDeliveryTime(timestamp: number): string {
  return DELIVERY_TIME_FORMATTER.format(new Date(timestamp * 1000));
}

/** Relative label for the composer pill, e.g. "Tomorrow, 9:00 AM". */
export function formatSchedulePill(timestamp: number): string {
  const now = Date.now();
  const target = new Date(timestamp * 1000);
  const deltaDays = Math.floor(
    (target.getTime() - now) / (24 * 60 * 60 * 1000),
  );
  if (deltaDays <= 0) {
    return new Intl.DateTimeFormat(undefined, {
      hour: "numeric",
      minute: "2-digit",
    }).format(target);
  }
  if (deltaDays === 1) {
    return new Intl.DateTimeFormat(undefined, {
      weekday: "short",
      hour: "numeric",
      minute: "2-digit",
    }).format(target);
  }
  return formatDeliveryTime(timestamp);
}
