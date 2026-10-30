/**
 * Parser for `/remind me <what> <when>` — the grammar in
 * `docs/slash-commands.md`. Mobile implements the same grammar
 * (`remind_time_parser.dart`); both apps must produce the same note and due
 * time for every vector in that spec.
 */

export type WallTime = {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  /** ISO weekday: 1 is Monday, 7 is Sunday. */
  weekday: number;
};

/**
 * Converts between instants (epoch milliseconds) and wall-clock time in one
 * time zone. Days, weeks and weekdays are wall-clock arithmetic, so they keep
 * the same local time across a DST change.
 */
export type RemindZone = {
  /**
   * The instant at which the zone's clock reads the given wall time. Fields
   * outside their normal range roll over, as in `Date.UTC`.
   */
  wallToInstant(
    year: number,
    month: number,
    day: number,
    hour: number,
    minute: number,
  ): number;
  instantToWall(instant: number): WallTime;
};

const ISO_WEEKDAYS: Record<string, number> = {
  Mon: 1,
  Tue: 2,
  Wed: 3,
  Thu: 4,
  Fri: 5,
  Sat: 6,
  Sun: 7,
};

/**
 * A zone backed by `Intl`. Omit `timeZone` for the device's zone; pass an IANA
 * name to pin one. Throws `RangeError` for an unknown name.
 */
export function createRemindZone(timeZone?: string): RemindZone {
  const formatter = new Intl.DateTimeFormat("en-US", {
    timeZone,
    hourCycle: "h23",
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "numeric",
    second: "numeric",
    weekday: "short",
  });

  const wallOf = (instant: number) => {
    const parts: Record<string, string> = {};
    for (const part of formatter.formatToParts(new Date(instant))) {
      parts[part.type] = part.value;
    }
    return {
      year: Number(parts.year),
      month: Number(parts.month),
      day: Number(parts.day),
      hour: Number(parts.hour),
      minute: Number(parts.minute),
      second: Number(parts.second),
      weekday: ISO_WEEKDAYS[parts.weekday],
    };
  };

  const offsetAt = (instant: number) => {
    const wall = wallOf(instant);
    const wallAsUtc = Date.UTC(
      wall.year,
      wall.month - 1,
      wall.day,
      wall.hour,
      wall.minute,
      wall.second,
    );
    return wallAsUtc - Math.floor(instant / 1_000) * 1_000;
  };

  return {
    wallToInstant(year, month, day, hour, minute) {
      const wallAsUtc = Date.UTC(year, month - 1, day, hour, minute);
      const guess = wallAsUtc - offsetAt(wallAsUtc);
      return wallAsUtc - offsetAt(guess);
    },
    instantToWall(instant) {
      const { second: _second, ...wall } = wallOf(instant);
      return wall;
    },
  };
}

export type RemindParseErrorKind =
  | "usage"
  | "notPersonal"
  | "missingNote"
  | "missingTime"
  | "ambiguousHour"
  | "invalidTime"
  | "outOfRange";

/**
 * The text to show above the usage line for each rejection; null shows the
 * usage line alone. Same copy as mobile.
 */
export const REMIND_PARSE_ERROR_MESSAGES: Record<
  RemindParseErrorKind,
  string | null
> = {
  usage: null,
  notPersonal: "Reminders are personal. Use /remind me …",
  missingNote: "Say what to be reminded about.",
  missingTime:
    "Say when, such as “in 20 minutes”, “at 3pm” or “tomorrow at 4pm”.",
  ambiguousHour: "Add am or pm to the hour, such as “at 3pm”.",
  invalidTime: "That is not a valid time.",
  outOfRange: "Pick a time within the next 365 days.",
};

export type RemindParseResult =
  | {
      ok: true;
      note: string;
      /** Due instant, epoch milliseconds. */
      dueAt: number;
    }
  | { ok: false; error: RemindParseErrorKind };

const MAX_LEAD_MS = 365 * 24 * 60 * 60 * 1_000;
const DEFAULT_HOUR = 9;
const MONDAY = 1;

const TIME_SOURCE = String.raw`(?:noon|midnight|\d{1,2}(?::\d{2})?(?:\s*(?:am|pm))?)`;
const WEEKDAY_NAMES = [
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
  "sunday",
];

const WHEN_FORMS = [
  String.raw`in\s+(?:\d+|an?)\s+(?:minutes?|mins?|hours?|hrs?|days?|weeks?)`,
  String.raw`at\s+${TIME_SOURCE}`,
  String.raw`tomorrow(?:\s+at\s+${TIME_SOURCE})?`,
  String.raw`(?:(?:on|next)\s+)?(?:${WEEKDAY_NAMES.join("|")})(?:\s+at\s+${TIME_SOURCE})?`,
  String.raw`next\s+week`,
];

const END_MATCHERS = WHEN_FORMS.map(
  (form) => new RegExp(String.raw`(?<!\S)(?:${form})$`, "i"),
);
const START_MATCHERS = WHEN_FORMS.map(
  (form) => new RegExp(String.raw`^(?:${form})(?=\s|$)`, "i"),
);

const ME_TARGET = /^me(?:\s+|$)/i;
const LEADING_TO = /^to(?:\s+|$)/i;
const IN_PATTERN = /^in\s+(\d+|an?)\s+(minute|min|hour|hr|day|week)s?$/i;
const TIME_PATTERN = /^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/i;
const AT_TAIL_PATTERN = new RegExp(String.raw`\s+at\s+(${TIME_SOURCE})$`, "i");

class Rejected extends Error {
  readonly kind: RemindParseErrorKind;

  constructor(kind: RemindParseErrorKind) {
    super(kind);
    this.kind = kind;
  }
}

/**
 * Parses the text after `/remind`. `now` is epoch milliseconds and `zone`
 * defaults to the device's.
 */
export function parseRemindCommand(
  args: string,
  { now, zone = createRemindZone() }: { now: number; zone?: RemindZone },
): RemindParseResult {
  const text = args.trim();
  if (text === "") return { ok: false, error: "usage" };
  const target = ME_TARGET.exec(text);
  if (!target) return { ok: false, error: "notPersonal" };
  const rest = text.slice(target[0].length).trim();
  if (rest === "") return { ok: false, error: "usage" };

  const split = splitWhen(rest);
  if (!split) return { ok: false, error: "missingTime" };
  const note = split.noteSource.trim().replace(LEADING_TO, "").trim();

  let dueAt: number;
  try {
    dueAt = resolveWhen(split.phrase, now, zone);
  } catch (error) {
    if (error instanceof Rejected) return { ok: false, error: error.kind };
    throw error;
  }
  if (note === "") return { ok: false, error: "missingNote" };
  if (dueAt <= now || dueAt - now > MAX_LEAD_MS) {
    return { ok: false, error: "outOfRange" };
  }
  return { ok: true, note, dueAt };
}

/**
 * Finds `<when>` at the end of `text`, else at the start; at each end the
 * longest phrase wins. Returns the phrase and the text outside it.
 */
function splitWhen(
  text: string,
): { phrase: string; noteSource: string } | null {
  const atEnd = longest(END_MATCHERS, text);
  if (atEnd) {
    return { phrase: atEnd[0], noteSource: text.slice(0, atEnd.index) };
  }
  const atStart = longest(START_MATCHERS, text);
  if (atStart) {
    return {
      phrase: atStart[0],
      noteSource: text.slice(atStart[0].length),
    };
  }
  return null;
}

function longest(matchers: RegExp[], text: string): RegExpExecArray | null {
  let best: RegExpExecArray | null = null;
  for (const matcher of matchers) {
    const match = matcher.exec(text);
    if (match && (!best || match[0].length > best[0].length)) best = match;
  }
  return best;
}

/**
 * The due instant for a matched `<when>` phrase; throws `Rejected` for a
 * phrase that matches the grammar but names no valid time.
 */
function resolveWhen(when: string, now: number, zone: RemindZone): number {
  const phrase = when.trim().toLowerCase();
  const wall = zone.instantToWall(now);

  const atWall = (dayOffset: number, hour: number, minute: number) =>
    zone.wallToInstant(
      wall.year,
      wall.month,
      wall.day + dayOffset,
      hour,
      minute,
    );

  const relative = IN_PATTERN.exec(phrase);
  if (relative) {
    const amount =
      relative[1] === "a" || relative[1] === "an"
        ? 1
        : Number.parseInt(relative[1], 10);
    if (!(amount >= 1)) throw new Rejected("invalidTime");
    switch (relative[2]) {
      case "minute":
      case "min":
        return now + amount * 60_000;
      case "hour":
      case "hr":
        return now + amount * 3_600_000;
      case "day":
        return atWall(amount, wall.hour, wall.minute);
      default:
        return atWall(amount * 7, wall.hour, wall.minute);
    }
  }

  if (phrase === "next week") {
    return atWall(daysAfterToday(wall, MONDAY), DEFAULT_HOUR, 0);
  }

  const timeTail = AT_TAIL_PATTERN.exec(phrase);
  let head: string;
  let clock: { hour: number; minute: number } | null = null;
  if (timeTail) {
    clock = parseClock(timeTail[1]);
    head = phrase.slice(0, timeTail.index);
  } else if (phrase.startsWith("at ")) {
    clock = parseClock(phrase.slice(3).trim());
    head = "";
  } else {
    head = phrase;
  }

  if (head === "") {
    const { hour, minute } = clock as { hour: number; minute: number };
    const today = atWall(0, hour, minute);
    return today > now ? today : atWall(1, hour, minute);
  }
  const hour = clock?.hour ?? DEFAULT_HOUR;
  const minute = clock?.minute ?? 0;
  if (head === "tomorrow") return atWall(1, hour, minute);

  const dayName = head.split(/\s+/).at(-1) ?? "";
  const weekday = WEEKDAY_NAMES.indexOf(dayName);
  if (weekday < 0) throw new Rejected("missingTime");
  return atWall(daysAfterToday(wall, weekday + 1), hour, minute);
}

function daysAfterToday(wall: WallTime, weekday: number): number {
  const days = (((weekday - wall.weekday) % 7) + 7) % 7;
  return days === 0 ? 7 : days;
}

function parseClock(source: string): { hour: number; minute: number } {
  if (source === "noon") return { hour: 12, minute: 0 };
  if (source === "midnight") return { hour: 0, minute: 0 };
  const match = TIME_PATTERN.exec(source);
  if (!match) throw new Rejected("invalidTime");
  const hour = Number.parseInt(match[1], 10);
  const minute = match[2] === undefined ? 0 : Number.parseInt(match[2], 10);
  const meridiem = match[3];
  if (minute > 59) throw new Rejected("invalidTime");
  if (meridiem === undefined) {
    if (match[2] === undefined) throw new Rejected("ambiguousHour");
    if (hour > 23) throw new Rejected("invalidTime");
    return { hour, minute };
  }
  if (hour < 1 || hour > 12) throw new Rejected("invalidTime");
  return { hour: (hour % 12) + (meridiem === "pm" ? 12 : 0), minute };
}

const DUE_DAY_FORMATTER = new Intl.DateTimeFormat("en-US", {
  weekday: "short",
  month: "short",
  day: "numeric",
});

const DUE_TIME_FORMATTER = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
});

/** `Fri, Oct 30 at 2:00 PM`, in the device's time zone. */
export function formatRemindDue(dueAt: number): string {
  const date = new Date(dueAt);
  const time = DUE_TIME_FORMATTER.format(date).replace(/\s/g, " ");
  return `${DUE_DAY_FORMATTER.format(date)} at ${time}`;
}
