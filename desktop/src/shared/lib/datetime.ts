/**
 * Relative date labels shared by the chat timeline and the Inbox.
 *
 * Both surfaces group items by calendar day and label the group. They used to
 * do it independently — chat via `messages/lib/dateFormatters.formatDayHeading`,
 * the Inbox inline inside `groupInboxItems` — and the two drifted apart: chat
 * appended an ordinal ("Monday, March 31st") while the Inbox always printed the
 * year ("Jul 8, 2026", even for a date three weeks ago).
 *
 * The ladder follows the Block writing standard for relative dates, with one
 * deliberate deviation noted on `formatDayGroupLabel`.
 */

const WEEKDAY_FORMATTER = new Intl.DateTimeFormat("en-US", {
  weekday: "long",
});

const WEEKDAY_MONTH_DAY_FORMATTER = new Intl.DateTimeFormat("en-US", {
  weekday: "long",
  month: "long",
  day: "numeric",
});

const MONTH_DAY_YEAR_FORMATTER = new Intl.DateTimeFormat("en-US", {
  month: "long",
  day: "numeric",
  year: "numeric",
});

const SHORT_WEEKDAY_SHORT_MONTH_DAY_FORMATTER = new Intl.DateTimeFormat(
  "en-US",
  {
    weekday: "short",
    month: "short",
    day: "numeric",
  },
);

const SHORT_MONTH_DAY_YEAR_FORMATTER = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

const TIME_FORMATTER = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
});

/** Days in a week, past which the weekday name stops being unambiguous. */
const WEEKDAY_BAND_DAYS = 7;

/**
 * Label for a group of items that share a calendar day — a chat day divider or
 * an Inbox section header.
 *
 * ```
 * Today          → "Today"
 * Yesterday      → "Yesterday"
 * 2–6 days ago   → "Monday"
 * older, this year → "Saturday, June 20"
 * earlier years  → "June 20, 2025"
 * ```
 *
 * The weekday rides along within the current year — feedback was that it still
 * orients ("was that a weekend?") well past the six-day band. Beyond a year it
 * stops earning its width: nobody maps "Friday" to anything a year later, and
 * the year itself needs the room.
 *
 * Deliberate deviation from the standard: the standard collapses anything over
 * ten months old to month and year ("Aug 2022"). A group label has to *identify*
 * its day — collapsing would give every day in a month the same header, so
 * scrolling old history would show a run of identical dividers with no way to
 * tell one day from the next. The day is kept and only the year is conditional.
 *
 * No ordinal suffix ("June 20", never "June 20th"), per the standard.
 *
 * `nowSeconds` is injectable so the relative bands are testable; it must stay a
 * parameter rather than a captured constant, because a label rendered before
 * midnight has to say "Yesterday" once the day rolls over.
 */
export function formatDayGroupLabel(
  unixSeconds: number,
  nowSeconds = Date.now() / 1_000,
): string {
  const date = new Date(unixSeconds * 1_000);
  const now = new Date(nowSeconds * 1_000);
  const dayDiff = calendarDaysBetween(now, date);

  if (dayDiff === 0) return "Today";
  if (dayDiff === 1) return "Yesterday";
  // Bounded below as well as above: a timestamp in the future (clock skew, or a
  // relay ahead of this machine) must not be labelled with a weekday that reads
  // as the recent past.
  if (dayDiff > 1 && dayDiff < WEEKDAY_BAND_DAYS) {
    return WEEKDAY_FORMATTER.format(date);
  }

  return date.getFullYear() === now.getFullYear()
    ? WEEKDAY_MONTH_DAY_FORMATTER.format(date)
    : MONTH_DAY_YEAR_FORMATTER.format(date);
}

/**
 * Label for a single item's timestamp — an Inbox list row, or a message in the
 * Inbox thread pane.
 *
 * ```
 * withTime: false (narrow rows)   withTime: true (roomy rows)
 * Today   → "2:34 PM"             Today   → "2:34 PM"
 * Yest.   → "Yesterday"           Yest.   → "Yesterday at 2:34 PM"
 * 2–6d    → "Monday"              2–6d    → "Monday at 2:34 PM"
 * year    → "Sat, Jun 20"         year    → "Sat, Jun 20 at 2:34 PM"
 * older   → "Jun 20, 2025"        older   → "Jun 20, 2025 at 2:34 PM"
 * ```
 *
 * The weekday stays through the current year (abbreviated, matching the month)
 * and drops once the year appears — see `formatDayGroupLabel` for why.
 *
 * Today needs no date word in either mode: a bare clock time already reads as
 * today, and "Today at 2:34 PM" is longer without saying more.
 *
 * `withTime` is a surface decision, not a preference. Somewhere you read
 * conversation, the time is part of the content, so pass `true`. In a narrow
 * list row it costs more width than it earns and the full timestamp is a hover
 * away, so pass `false`.
 *
 * Months are abbreviated here but spelled out in `formatDayGroupLabel` — a
 * day divider is a roomy header of its own, an item label shares a row with a
 * name, a channel, and a preview.
 */
export function formatItemTimestamp(
  unixSeconds: number,
  {
    withTime = false,
    nowSeconds = Date.now() / 1_000,
  }: { withTime?: boolean; nowSeconds?: number } = {},
): string {
  const date = new Date(unixSeconds * 1_000);
  const now = new Date(nowSeconds * 1_000);
  const dayDiff = calendarDaysBetween(now, date);
  const time = TIME_FORMATTER.format(date);

  if (dayDiff === 0) return time;

  let dayLabel: string;
  if (dayDiff === 1) {
    dayLabel = "Yesterday";
  } else if (dayDiff > 1 && dayDiff < WEEKDAY_BAND_DAYS) {
    dayLabel = WEEKDAY_FORMATTER.format(date);
  } else {
    dayLabel =
      date.getFullYear() === now.getFullYear()
        ? SHORT_WEEKDAY_SHORT_MONTH_DAY_FORMATTER.format(date)
        : SHORT_MONTH_DAY_YEAR_FORMATTER.format(date);
  }

  return withTime ? `${dayLabel} at ${time}` : dayLabel;
}

/** Local midnight of the calendar day containing `date`. */
function startOfLocalDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/**
 * Whole calendar days from `date` to `now`, in local time. Rounded rather than
 * floored so a DST transition — a 23- or 25-hour day — still counts as one day.
 */
function calendarDaysBetween(now: Date, date: Date): number {
  return Math.round(
    (startOfLocalDay(now).getTime() - startOfLocalDay(date).getTime()) /
      86_400_000,
  );
}
