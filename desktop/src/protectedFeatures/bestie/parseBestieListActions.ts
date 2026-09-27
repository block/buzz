import type { BestieListAddInput, BestieListKind } from "./bestieListTypes";

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


function coerceDueAt(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.floor(value);
  }
  if (typeof value === "string" && value.trim().length > 0) {
    const asNumber = Number(value);
    if (Number.isFinite(asNumber) && String(asNumber) === value.trim()) {
      return Math.floor(asNumber);
    }
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return Math.floor(parsed / 1000);
  }
  return null;
}

function parseAddItems(value: unknown): BestieListAddInput[] {
  if (!Array.isArray(value)) return [];
  const items: BestieListAddInput[] = [];
  for (const entry of value) {
    if (typeof entry !== "object" || entry === null) continue;
    const record = entry as Record<string, unknown>;
    if (!isKind(record.kind) || typeof record.text !== "string") continue;
    const text = record.text.trim();
    if (!text) continue;
    const dueAt = coerceDueAt(record.dueAt);
    items.push({
      dueAt: record.kind === "reminder" ? dueAt : null,
      kind: record.kind,
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
