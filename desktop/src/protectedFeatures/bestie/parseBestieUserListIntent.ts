import type { BestieListAddInput, BestieListKind } from "./bestieListTypes";

/**
 * Natural-language intents from the *user* in Bestie DM / popover.
 * Applied client-side so list mutations work even when the agent never emits a fence.
 */
export type BestieUserListIntent =
  | { items: BestieListAddInput[]; op: "add" }
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
const AT_TIME_RE =
  /\b(?:at|by)\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?\b/i;
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
 * Parse a due time from free text. Returns unix seconds or null.
 * Mutates nothing; callers may strip the matched phrase from the text.
 */
export function parseBestieDueAtFromText(
  text: string,
  nowMs = Date.now(),
): { dueAt: number | null; textWithoutDue: string } {
  const inMatch = text.match(IN_DURATION_RE);
  if (inMatch) {
    const amount = Number(inMatch[1]);
    const seconds = durationSeconds(amount, inMatch[2] ?? "m");
    const dueAt = Math.floor(nowMs / 1000) + seconds;
    const textWithoutDue = text.replace(inMatch[0], " ").replace(/\s+/g, " ").trim();
    return { dueAt, textWithoutDue };
  }

  const tomorrow = TOMORROW_RE.test(text);
  const today = TODAY_RE.test(text);
  const atMatch = text.match(AT_TIME_RE);
  if (atMatch) {
    let hours = Number(atMatch[1]);
    const minutes = atMatch[2] != null ? Number(atMatch[2]) : 0;
    const meridiem = (atMatch[3] ?? "").toLowerCase();
    if (meridiem === "pm" && hours < 12) hours += 12;
    if (meridiem === "am" && hours === 12) hours = 0;

    const base = new Date(nowMs);
    if (tomorrow) base.setDate(base.getDate() + 1);
    base.setSeconds(0, 0);
    base.setHours(hours, minutes, 0, 0);
    // If "at 3pm" with no day and that time already passed today, roll to tomorrow.
    if (!tomorrow && !today && base.getTime() <= nowMs) {
      base.setDate(base.getDate() + 1);
    }
    let cleaned = text.replace(atMatch[0], " ");
    cleaned = cleaned.replace(TOMORROW_RE, " ").replace(TODAY_RE, " ");
    cleaned = cleaned.replace(/\s+/g, " ").trim();
    return { dueAt: Math.floor(base.getTime() / 1000), textWithoutDue: cleaned };
  }

  if (tomorrow) {
    const base = new Date(nowMs);
    base.setDate(base.getDate() + 1);
    base.setHours(9, 0, 0, 0);
    const cleaned = text.replace(TOMORROW_RE, " ").replace(/\s+/g, " ").trim();
    return { dueAt: Math.floor(base.getTime() / 1000), textWithoutDue: cleaned };
  }

  return { dueAt: null, textWithoutDue: text.trim() };
}

function trimTrailingPunctuation(value: string): string {
  return value.replace(/[.\s]+$/g, "").trim();
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
    const { dueAt, textWithoutDue } = parseBestieDueAtFromText(raw, nowMs);
    const text = trimTrailingPunctuation(textWithoutDue);
    if (!text) return null;
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
