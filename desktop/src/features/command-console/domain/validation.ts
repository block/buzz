export type JsonPrimitive = boolean | number | string | null;
export type JsonValue =
  | JsonPrimitive
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

export type UnknownRecord = Record<string, unknown>;

const MAX_JSON_DEPTH = 64;
const MAX_JSON_NODES = 10_000;
const RFC3339 =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?(?:Z|([+-])(\d{2}):(\d{2}))$/;
const SHA256 = /^sha256:[0-9a-f]{64}$/;

export function isRecord(value: unknown): value is UnknownRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

export function hasExactKeys(
  value: UnknownRecord,
  keys: readonly string[],
): boolean {
  const actual = Object.keys(value);
  return (
    actual.length === keys.length && actual.every((key) => keys.includes(key))
  );
}

export function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

export function isCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function daysInMonth(year: number, month: number): number {
  if (month === 2) {
    const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
    return leap ? 29 : 28;
  }
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

export function isRfc3339(value: unknown): value is string {
  if (typeof value !== "string") return false;
  const match = RFC3339.exec(value);
  if (!match) return false;
  const [
    ,
    yearRaw,
    monthRaw,
    dayRaw,
    hourRaw,
    minuteRaw,
    secondRaw,
    ,
    offsetHourRaw,
    offsetMinuteRaw,
  ] = match;
  const year = Number(yearRaw);
  const month = Number(monthRaw);
  const day = Number(dayRaw);
  const hour = Number(hourRaw);
  const minute = Number(minuteRaw);
  const second = Number(secondRaw);
  const offsetHour = offsetHourRaw === undefined ? 0 : Number(offsetHourRaw);
  const offsetMinute =
    offsetMinuteRaw === undefined ? 0 : Number(offsetMinuteRaw);
  return (
    month >= 1 &&
    month <= 12 &&
    day >= 1 &&
    day <= daysInMonth(year, month) &&
    hour <= 23 &&
    minute <= 59 &&
    second <= 59 &&
    offsetHour <= 23 &&
    offsetMinute <= 59
  );
}

export function isHash(value: unknown): value is string {
  return typeof value === "string" && SHA256.test(value);
}

export function parseTextArray(value: unknown): readonly string[] | null {
  if (!Array.isArray(value) || !value.every(isText)) return null;
  return Object.freeze([...value]);
}

export function parseObjectArray<T>(
  value: unknown,
  parser: (item: unknown) => T | null,
): readonly T[] | null {
  if (!Array.isArray(value)) return null;
  const parsed: T[] = [];
  for (const item of value) {
    const result = parser(item);
    if (result === null) return null;
    parsed.push(result);
  }
  return Object.freeze(parsed);
}

export function required<T>(parsed: T | null, kind: string): T {
  if (parsed === null) throw new TypeError(`Invalid ${kind} contract input.`);
  return parsed;
}

export function classificationIsSafe(
  classification: Classification,
  nested: readonly Classification[],
): boolean {
  return resolveClassification(classification, nested) === classification;
}

export function parseQuotedLocation(
  value: unknown,
): { readonly quote: string; readonly location: string } | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["quote", "location"]) ||
    !isText(value.quote) ||
    !isText(value.location)
  ) {
    return null;
  }
  return Object.freeze({ quote: value.quote, location: value.location });
}

export function isApprovalState(
  value: unknown,
): value is "pending" | "approved" | "rejected" {
  return value === "pending" || value === "approved" || value === "rejected";
}

export function parseActionDetail(
  value: unknown,
  keys: readonly string[],
): UnknownRecord | null {
  return isRecord(value) && hasExactKeys(value, keys) ? value : null;
}

export type JsonCloneResult =
  | { readonly ok: true; readonly value: JsonValue }
  | { readonly ok: false };

/**
 * Clones and freezes JSON without recursion, rejecting cycles and data beyond
 * the persisted-data depth/node budget.
 */
export function cloneBoundedJson(root: unknown): JsonCloneResult {
  try {
    const primitive = (value: unknown): value is JsonPrimitive =>
      value === null ||
      typeof value === "string" ||
      typeof value === "boolean" ||
      (typeof value === "number" && Number.isFinite(value));
    if (primitive(root)) return { ok: true, value: root };
    if (!Array.isArray(root) && !isRecord(root)) return { ok: false };

    const rootOutput: unknown[] | UnknownRecord = Array.isArray(root)
      ? []
      : Object.create(null);
    const seen = new WeakSet<object>([root]);
    const containers: object[] = [rootOutput];
    const stack: Array<{
      input: unknown[] | UnknownRecord;
      output: unknown[] | UnknownRecord;
      depth: number;
    }> = [{ input: root, output: rootOutput, depth: 0 }];
    let nodes = 1;

    while (stack.length > 0) {
      const frame = stack.pop();
      if (!frame) return { ok: false };
      const entries: Array<[string, unknown]> = Array.isArray(frame.input)
        ? Array.from(frame.input, (item, index) => [String(index), item])
        : Object.entries(frame.input);
      if (nodes + entries.length > MAX_JSON_NODES) return { ok: false };
      nodes += entries.length;

      for (const [key, item] of entries) {
        const childDepth = frame.depth + 1;
        if (childDepth > MAX_JSON_DEPTH) return { ok: false };
        if (primitive(item)) {
          (frame.output as UnknownRecord)[key] = item;
          continue;
        }
        if (!Array.isArray(item) && !isRecord(item)) return { ok: false };
        if (seen.has(item)) return { ok: false };
        seen.add(item);
        const output: unknown[] | UnknownRecord = Array.isArray(item)
          ? []
          : Object.create(null);
        (frame.output as UnknownRecord)[key] = output;
        containers.push(output);
        stack.push({ input: item, output, depth: childDepth });
      }
    }
    for (let index = containers.length - 1; index >= 0; index -= 1) {
      Object.freeze(containers[index]);
    }
    return { ok: true, value: rootOutput as JsonValue };
  } catch {
    return { ok: false };
  }
}
import { resolveClassification } from "./classification";
import type { Classification } from "./classification";
