import type {
  BestieListAddInput,
  BestieListKind,
  BestieReminderRepeat,
} from "./bestieListTypes";
import { crystallizeReminderText } from "./parseBestieUserListIntent";

/**
 * Structured Bestie list mutations embedded in agent chat.
 *
 * Prefer a fenced block so free-form chat doesn't accidentally mutate lists:
 *
 * ```bestie-list
 * {"op":"add","items":[{"kind":"todo","text":"Ship phase 2"}]}
 * ```
 *
 * Also accepts a single JSON object (or array of objects) without a fence when
 * the whole message trims to JSON starting with `"op"`.
 */
export type BestieListAction =
  | {
      items: BestieListAddInput[];
      op: "add";
    }
  | {
      id: string;
      op: "complete" | "remove";
    };

const FENCE_RE = /```bestie-list\s*\r?\n([\s\S]*?)```/gi;

function isKind(value: unknown): value is BestieListKind {
  return value === "todo" || value === "reminder";
}

function parseRepeat(value: unknown): BestieReminderRepeat | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.kind === "daily") return { kind: "daily" };
  if (record.kind === "weekly") {
    const weekday =
      typeof record.weekday === "number" && Number.isFinite(record.weekday)
        ? Math.floor(record.weekday)
        : null;
    if (weekday == null) return null;
    return { kind: "weekly", weekday: ((weekday % 7) + 7) % 7 };
  }
  return null;
}

/**
 * Normalize fence dueAt to unix **seconds** for storage.
 *
 * Teach agents unix **milliseconds** (Date.now()-style). On apply:
 * - ms-scale (≥ 1e12) → divide to seconds
 * - seconds-scale (< 1e12, e.g. 1790611740) → keep as seconds (coerce path
 *   that used to store raw ms broke fire time vs nowSeconds)
 * ISO strings parse to seconds.
 */
export function coerceDueAt(value: unknown): number | null {
  let raw: number | null = null;
  if (typeof value === "number" && Number.isFinite(value)) {
    raw = value;
  } else if (typeof value === "string" && value.trim().length > 0) {
    const asNumber = Number(value);
    if (Number.isFinite(asNumber) && String(asNumber) === value.trim()) {
      raw = asNumber;
    } else {
      const parsed = Date.parse(value);
      if (Number.isFinite(parsed)) return Math.floor(parsed / 1000);
    }
  }
  if (raw == null || !Number.isFinite(raw) || raw < 0) return null;
  // Ms → seconds when agent followed "teach fence ms".
  if (raw >= 1_000_000_000_000) {
    return Math.floor(raw / 1000);
  }
  // Seconds (legacy / mistaken unit) — keep; do not *1000 into storage.
  return Math.floor(raw);
}

function parseAddItems(value: unknown): BestieListAddInput[] {
  if (!Array.isArray(value)) return [];
  const items: BestieListAddInput[] = [];
  for (const entry of value) {
    if (typeof entry !== "object" || entry === null) continue;
    const record = entry as Record<string, unknown>;
    if (!isKind(record.kind) || typeof record.text !== "string") continue;
    const rawText = record.text.trim();
    if (!rawText) continue;
    const text =
      record.kind === "reminder"
        ? crystallizeReminderText(rawText)
        : rawText;
    if (!text) continue;
    const dueAt = coerceDueAt(record.dueAt);
    items.push({
      dueAt: record.kind === "reminder" ? dueAt : null,
      kind: record.kind,
      repeat: record.kind === "reminder" ? parseRepeat(record.repeat) : null,
      text,
    });
  }
  return items;
}

export function parseBestieListActionPayload(
  value: unknown,
): BestieListAction[] {
  const payloads = Array.isArray(value) ? value : [value];
  const actions: BestieListAction[] = [];
  for (const payload of payloads) {
    if (typeof payload !== "object" || payload === null) continue;
    const record = payload as Record<string, unknown>;
    if (record.op === "add") {
      const items = parseAddItems(record.items);
      if (items.length > 0) actions.push({ items, op: "add" });
      continue;
    }
    if (
      (record.op === "complete" || record.op === "remove") &&
      typeof record.id === "string" &&
      record.id.length > 0
    ) {
      actions.push({ id: record.id, op: record.op });
    }
  }
  return actions;
}

function tryParseJson(raw: string): unknown | null {
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

/** Extract Bestie list actions from an agent message body. */
export function parseBestieListActionsFromMessage(
  content: string,
): BestieListAction[] {
  const actions: BestieListAction[] = [];
  const fences = content.matchAll(FENCE_RE);
  for (const match of fences) {
    const json = tryParseJson(match[1]?.trim() ?? "");
    if (json != null) {
      actions.push(...parseBestieListActionPayload(json));
    }
  }
  if (actions.length > 0) return actions;

  const trimmed = content.trim();
  if (
    (trimmed.startsWith("{") || trimmed.startsWith("[")) &&
    trimmed.includes('"op"')
  ) {
    const json = tryParseJson(trimmed);
    if (json != null) return parseBestieListActionPayload(json);
  }
  return [];
}
