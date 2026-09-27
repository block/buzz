import type { BestieListKind } from "./bestieListTypes";

/** Omit empty counts the same way Projects overview does. */
export function presentBestieContextCount(
  value: number | undefined,
): number | undefined {
  return value != null && value > 0 ? value : undefined;
}

export function bestieCategoryTitle(kind: BestieListKind): string {
  return kind === "reminder" ? "Reminders" : "To-dos";
}

/**
 * IdleAuxiliary (slide) is only for a drilled-in category’s item list.
 * Categories themselves live in the fixed RHS column — never the slide.
 * Returns null when the slide must stay closed (category home).
 */
export function bestieIdleAuxiliaryKind(
  activeKind: BestieListKind | null,
): BestieListKind | null {
  return activeKind;
}

/** Convert datetime-local value to unix seconds, or null if empty/invalid. */
export function dueAtFromDatetimeLocal(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const ms = Date.parse(trimmed);
  if (!Number.isFinite(ms)) return null;
  return Math.floor(ms / 1000);
}

/** Format unix seconds for a datetime-local input (local timezone). */
export function datetimeLocalFromDueAt(dueAt: number): string {
  const date = new Date(dueAt * 1000);
  if (!Number.isFinite(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
