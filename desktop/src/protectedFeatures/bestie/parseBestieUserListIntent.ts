import type { BestieListAddInput, BestieListKind } from "./bestieListTypes";

/**
 * Natural-language intents from the *user* in Bestie DM / popover.
 * Applied client-side so list mutations work even when the agent never emits a fence.
 *
 * Unambiguous reminders (relative duration, clock with am/pm, 24h hour) auto-add.
 * Bare 1–12 clocks without am/pm do **not** auto-add — confirm AM vs PM (or next
 * occurrence) via agent teach/fence first.
 */
export type BestieBareClock = {
  /** 1–12 clock face hour (as spoken; not yet resolved to 24h). */
  hour: number;
  minute: number;
  dayHint: "today" | "tomorrow" | null;
};

export type BestieUserListIntent =
  | { items: BestieListAddInput[]; op: "add" }
  | {
      /** Parsed task + bare clock; client must NOT create until am/pm confirmed. */
      bareClock: BestieBareClock;
      op: "reminder-confirm-needed";
      text: string;
    }
  | { kind?: BestieListKind; op: "complete-match"; text: string }
  | { kind?: BestieListKind; op: "remove-match"; text: string };

const ADD_REMINDER_RE =
  /^(?:please\s+)?(?:remind\s+me\s+to|add\s+(?:a\s+)?reminder(?:\s+to|\s+for)?|set\s+(?:a\s+)?reminder(?:\s+to|\s+for)?)\s+(.+)$/i;
const ADD_TODO_RE =
  /^(?:please\s+)?(?:add\s+(?:a\s+)?(?:to-?do|todo|task)|to-?do|todo)\s*:?\s+(.+)$/i;
const COMPLETE_RE =
  /^(?:please\s+)?(?:mark|complete|finish|done\s+with|check\s+off)\s+(?:the\s+)?(?:(reminder|to-?do|todo|task)\s+)?(?:called\s+|named\s+)?["']?(.+?)["']?\s*(?:as\s+done|done)?\.?$/i;
const REMOVE_RE =
  /^(?:please\s+)?(?:remove|delete|cancel|clear)\s+(?:the\s+)?(?:(reminder|to-?do|todo|task)\s+)?(?:called\s+|named\s+)?["']?(.+?)["']?\.?$/i;

const IN_DURATION_RE =
  /\b(?:in|after)\s+(\d+)\s*(minutes?|mins?|m|hours?|hrs?|h|days?|d)\b/i;
const AT_TIME_RE = /\b(?:at|by)\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?\b/i;
/** Leading bare clock: "8:36 to finish…" / "8:36pm finish…" */
const LEADING_CLOCK_RE =
  /^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?\s*(?:to\s+|for\s+)?/i;
const TOMORROW_RE = /\btomorrow\b/i;
const TODAY_RE = /\btoday\b/i;

function kindFromLabel(label: string | undefined): BestieListKind | undefined {
  if (!label) return undefined;
  const lower = label.toLowerCase();
  if (lower.startsWith("remind")) return "reminder";
  if (lower.startsWith("to") || lower === "task") return "todo";
  return undefined;
}

function durationSeconds(amount: number, unit: string): number {
  const u = unit.toLowerCase();
  if (u.startsWith("m")) return amount * 60;
  if (u.startsWith("h")) return amount * 3600;
  return amount * 86400;
}

/**
 * Crystallize reminder display text: drop a leftover leading "to"/"for", trim
 * trailing punctuation, and capitalize the first letter so
 * "remind me to run the nightly report" → stored "Run the nightly report".
 */
export function crystallizeReminderText(text: string): string {
  let s = text.trim();
  s = s.replace(/^(?:to|for)\s+/i, "");
  s = s.replace(/[.\s]+$/g, "").trim();
  if (!s) return s;
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function trimTrailingPunctuation(value: string): string {
  return value.replace(/[.\s]+$/g, "").trim();
}

function dayHintFromText(text: string): "today" | "tomorrow" | null {
  if (TOMORROW_RE.test(text)) return "tomorrow";
  if (TODAY_RE.test(text)) return "today";
  return null;
}

/** True when hour is a 12-hour face value that needs am/pm to disambiguate. */
function isAmbiguousTwelveHourFace(hour: number, meridiem: string): boolean {
  if (meridiem === "am" || meridiem === "pm") return false;
  // 13–23 are unambiguous 24h; 0 is unambiguous midnight.
  return hour >= 1 && hour <= 12;
}

function resolveHours(hour: number, meridiem: string): number {
  let hours = hour;
  if (meridiem === "pm" && hours < 12) hours += 12;
  if (meridiem === "am" && hours === 12) hours = 0;
  return hours;
}

function buildDueAtFromClock(
  hours24: number,
  minutes: number,
  dayHint: "today" | "tomorrow" | null,
  nowMs: number,
): number {
  const base = new Date(nowMs);
  if (dayHint === "tomorrow") base.setDate(base.getDate() + 1);
  base.setSeconds(0, 0);
  base.setHours(hours24, minutes, 0, 0);
  // If no day and that time already passed today, roll to tomorrow.
  if (dayHint == null && base.getTime() <= nowMs) {
    base.setDate(base.getDate() + 1);
  }
  return Math.floor(base.getTime() / 1000);
}

export type BestieMeridiem = "am" | "pm";

/**
 * Resolve a bare 1–12 clock + confirmed am/pm into unix seconds (local tz).
 * Rolls to tomorrow when that clock today has already passed.
 */
export function resolveBestieBareClockDueAt(
  bareClock: BestieBareClock,
  meridiem: BestieMeridiem,
  nowMs = Date.now(),
): number {
  const hours24 = resolveHours(bareClock.hour, meridiem);
  return buildDueAtFromClock(
    hours24,
    bareClock.minute,
    bareClock.dayHint,
    nowMs,
  );
}

function normalizeMeridiemToken(raw: string): BestieMeridiem | null {
  const s = raw.toLowerCase().replace(/\./g, "").replace(/\s+/g, "");
  if (s === "am" || s === "a") return "am";
  if (s === "pm" || s === "p") return "pm";
  return null;
}

/**
 * Parse a short AM/PM confirm reply (e.g. "PM", "8:45 PM", "am please").
 * Returns null when the message is not a clear meridiem confirm.
 */
export function parseBestieReminderMeridiemReply(
  content: string,
): { meridiem: BestieMeridiem; hour?: number; minute?: number } | null {
  const trimmed = content.trim();
  if (!trimmed || trimmed.length > 80) return null;
  const firstLine = trimmed.split(/\n/)[0]?.trim() ?? trimmed;

  // Bare "PM" / "AM" / "p.m." / "a.m."
  const bare = firstLine.match(
    /^(?:please\s+)?(a\.?\s*m\.?|p\.?\s*m\.?|am|pm)\s*(?:please|thanks|thank you)?\.?$/i,
  );
  if (bare) {
    const meridiem = normalizeMeridiemToken(bare[1] ?? "");
    if (meridiem) return { meridiem };
  }

  // "8:45 PM" / "at 8:45pm" / "8 PM"
  const clock = firstLine.match(
    /^(?:please\s+)?(?:at\s+|make\s+it\s+)?(\d{1,2})(?::(\d{2}))?\s*(a\.?\s*m\.?|p\.?\s*m\.?|am|pm)\s*(?:please|thanks|thank you)?\.?$/i,
  );
  if (clock) {
    const meridiem = normalizeMeridiemToken(clock[3] ?? "");
    if (!meridiem) return null;
    const hour = Number(clock[1]);
    if (!Number.isFinite(hour) || hour < 0 || hour > 23) return null;
    const minute = clock[2] != null ? Number(clock[2]) : undefined;
    return {
      meridiem,
      hour,
      ...(minute != null && Number.isFinite(minute) ? { minute } : {}),
    };
  }

  // "go with PM" / "make it PM"
  const goWith = firstLine.match(
    /^(?:go\s+with|make\s+it|it(?:'|’)s)\s+(a\.?\s*m\.?|p\.?\s*m\.?|am|pm)\.?$/i,
  );
  if (goWith) {
    const meridiem = normalizeMeridiemToken(goWith[1] ?? "");
    if (meridiem) return { meridiem };
  }

  return null;
}

/**
 * Find am/pm stated next to a known clock face in agent (or user) prose.
 * Example: "Got it — 8:45 PM" → "pm".
 */
export function extractStatedMeridiemForBareClock(
  content: string,
  bareClock: BestieBareClock,
): BestieMeridiem | null {
  const hour = bareClock.hour;
  const minute = bareClock.minute;
  const mm = String(minute).padStart(2, "0");
  const patterns = [
    new RegExp(
      `\\b${hour}:${mm}\\s*(a\\.?\\s*m\\.?|p\\.?\\s*m\\.?|am|pm)\\b`,
      "i",
    ),
    new RegExp(
      `\\b${hour}\\s*(a\\.?\\s*m\\.?|p\\.?\\s*m\\.?|am|pm)\\b`,
      "i",
    ),
  ];
  for (const re of patterns) {
    const match = content.match(re);
    if (!match) continue;
    const meridiem = normalizeMeridiemToken(match[1] ?? "");
    if (meridiem) return meridiem;
  }
  return null;
}

/** True when local wall-clock of dueAt matches the confirmed bare clock + meridiem. */
export function dueAtMatchesBareClockMeridiem(
  dueAt: number,
  bareClock: BestieBareClock,
  meridiem: BestieMeridiem,
): boolean {
  const date = new Date(dueAt * 1000);
  const expected = resolveHours(bareClock.hour, meridiem);
  return date.getHours() === expected && date.getMinutes() === bareClock.minute;
}

/**
 * Infer a bare clock from prose that already includes am/pm (e.g. "8:45 PM").
 * Used to repair agent fences when pending confirm was cleared or missing.
 */
export function inferBareClockFromStatedMeridiemClock(
  content: string,
): { bareClock: BestieBareClock; meridiem: BestieMeridiem } | null {
  const match = content.match(
    /\b(\d{1,2}):(\d{2})\s*(a\.?\s*m\.?|p\.?\s*m\.?|am|pm)\b/i,
  );
  if (!match) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  const meridiem = normalizeMeridiemToken(match[3] ?? "");
  if (!meridiem || !Number.isFinite(hour) || !Number.isFinite(minute)) {
    return null;
  }
  if (hour < 1 || hour > 12 || minute < 0 || minute > 59) return null;
  return {
    bareClock: { dayHint: null, hour, minute },
    meridiem,
  };
}

/**
 * When agent prose states AM/PM for the pending bare clock but fence dueAt
 * disagrees (classic: text says PM, dueAt is next-morning AM), replace dueAt
 * with a client-resolved epoch.
 */
export function reconcileReminderDueAtWithStatedMeridiem(
  dueAt: number | null,
  content: string,
  bareClock: BestieBareClock | null,
  nowMs = Date.now(),
): number | null {
  let clock = bareClock;
  let stated: BestieMeridiem | null = clock
    ? extractStatedMeridiemForBareClock(content, clock)
    : null;
  if (!stated) {
    const inferred = inferBareClockFromStatedMeridiemClock(content);
    if (inferred) {
      clock = inferred.bareClock;
      stated = inferred.meridiem;
    }
  }
  if (!clock || !stated) return dueAt;
  const resolved = resolveBestieBareClockDueAt(clock, stated, nowMs);
  if (dueAt == null) return resolved;
  if (!dueAtMatchesBareClockMeridiem(dueAt, clock, stated)) {
    return resolved;
  }
  return dueAt;
}

function stripDayWords(text: string): string {
  return text
    .replace(TOMORROW_RE, " ")
    .replace(TODAY_RE, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export type ParseBestieDueAtResult = {
  /** Set when a 1–12 clock has no am/pm — caller must not auto-create. */
  ambiguousBareClock: BestieBareClock | null;
  dueAt: number | null;
  textWithoutDue: string;
};

/**
 * Parse a due time from free text. Returns unix seconds or null.
 * Bare 1–12 clocks without am/pm set `ambiguousBareClock` and leave `dueAt` null.
 */
export function parseBestieDueAtFromText(
  text: string,
  nowMs = Date.now(),
): ParseBestieDueAtResult {
  const inMatch = text.match(IN_DURATION_RE);
  if (inMatch) {
    const amount = Number(inMatch[1]);
    const seconds = durationSeconds(amount, inMatch[2] ?? "m");
    const dueAt = Math.floor(nowMs / 1000) + seconds;
    const textWithoutDue = text
      .replace(inMatch[0], " ")
      .replace(/\s+/g, " ")
      .trim();
    return { ambiguousBareClock: null, dueAt, textWithoutDue };
  }

  const dayHint = dayHintFromText(text);
  const atMatch = text.match(AT_TIME_RE);
  if (atMatch) {
    const faceHour = Number(atMatch[1]);
    const minutes = atMatch[2] != null ? Number(atMatch[2]) : 0;
    const meridiem = (atMatch[3] ?? "").toLowerCase();
    let cleaned = text.replace(atMatch[0], " ");
    cleaned = stripDayWords(cleaned);

    if (isAmbiguousTwelveHourFace(faceHour, meridiem)) {
      return {
        ambiguousBareClock: {
          dayHint,
          hour: faceHour,
          minute: minutes,
        },
        dueAt: null,
        textWithoutDue: cleaned,
      };
    }

    const hours24 = resolveHours(faceHour, meridiem);
    return {
      ambiguousBareClock: null,
      dueAt: buildDueAtFromClock(hours24, minutes, dayHint, nowMs),
      textWithoutDue: cleaned,
    };
  }

  const leading = text.match(LEADING_CLOCK_RE);
  if (leading) {
    const faceHour = Number(leading[1]);
    const minutes = leading[2] != null ? Number(leading[2]) : 0;
    const meridiem = (leading[3] ?? "").toLowerCase();
    // Only treat as a clock lead-in when it looks like time (has :mm or am/pm).
    const hasMinutes = leading[2] != null;
    const hasMeridiem = meridiem === "am" || meridiem === "pm";
    if (hasMinutes || hasMeridiem) {
      let cleaned = text.slice(leading[0].length);
      cleaned = stripDayWords(cleaned);
      if (isAmbiguousTwelveHourFace(faceHour, meridiem)) {
        return {
          ambiguousBareClock: {
            dayHint,
            hour: faceHour,
            minute: minutes,
          },
          dueAt: null,
          textWithoutDue: cleaned,
        };
      }
      const hours24 = resolveHours(faceHour, meridiem);
      return {
        ambiguousBareClock: null,
        dueAt: buildDueAtFromClock(hours24, minutes, dayHint, nowMs),
        textWithoutDue: cleaned,
      };
    }
  }

  if (dayHint === "tomorrow") {
    const base = new Date(nowMs);
    base.setDate(base.getDate() + 1);
    base.setHours(9, 0, 0, 0);
    const cleaned = stripDayWords(text);
    return {
      ambiguousBareClock: null,
      dueAt: Math.floor(base.getTime() / 1000),
      textWithoutDue: cleaned,
    };
  }

  return {
    ambiguousBareClock: null,
    dueAt: null,
    textWithoutDue: text.trim(),
  };
}

/**
 * Extract zero or one list intent from a user message body.
 * Conservative: only matches clear imperative shapes.
 */
export function parseBestieUserListIntent(
  content: string,
  nowMs = Date.now(),
): BestieUserListIntent | null {
  const trimmed = content.trim();
  if (!trimmed || trimmed.length > 500) return null;

  // Prefer a single line / first sentence for matching.
  const firstLine = trimmed.split(/\n/)[0]?.trim() ?? trimmed;

  const reminderMatch = firstLine.match(ADD_REMINDER_RE);
  if (reminderMatch) {
    const raw = trimTrailingPunctuation(reminderMatch[1] ?? "");
    const { ambiguousBareClock, dueAt, textWithoutDue } =
      parseBestieDueAtFromText(raw, nowMs);
    const text = crystallizeReminderText(textWithoutDue);
    if (!text) return null;
    if (ambiguousBareClock) {
      return {
        bareClock: ambiguousBareClock,
        op: "reminder-confirm-needed",
        text,
      };
    }
    return {
      items: [{ dueAt, kind: "reminder", text }],
      op: "add",
    };
  }

  const todoMatch = firstLine.match(ADD_TODO_RE);
  if (todoMatch) {
    const text = trimTrailingPunctuation(todoMatch[1] ?? "");
    if (!text) return null;
    return {
      items: [{ kind: "todo", text }],
      op: "add",
    };
  }

  const completeMatch = firstLine.match(COMPLETE_RE);
  if (completeMatch) {
    const text = trimTrailingPunctuation(completeMatch[2] ?? "");
    if (!text) return null;
    return {
      kind: kindFromLabel(completeMatch[1]),
      op: "complete-match",
      text,
    };
  }

  const removeMatch = firstLine.match(REMOVE_RE);
  if (removeMatch) {
    const text = trimTrailingPunctuation(removeMatch[2] ?? "");
    if (!text) return null;
    return {
      kind: kindFromLabel(removeMatch[1]),
      op: "remove-match",
      text,
    };
  }

  return null;
}

/** True when the message should carry a Bestie list turn hint for the agent. */
export function messageLooksLikeBestieListRequest(content: string): boolean {
  return parseBestieUserListIntent(content) != null;
}

/** True when NL reminder needs AM/PM (or next-occurrence) confirm before create. */
export function messageNeedsBestieReminderBareClockConfirm(
  content: string,
  nowMs = Date.now(),
): boolean {
  const intent = parseBestieUserListIntent(content, nowMs);
  return intent?.op === "reminder-confirm-needed";
}

/** True when the message is a short AM/PM confirm reply (pending confirm path). */
export function messageLooksLikeBestieReminderMeridiemReply(
  content: string,
): boolean {
  return parseBestieReminderMeridiemReply(content) != null;
}
