import type { BestieJobState } from "./bestieJobTypes";
import { enabledJobs } from "./bestieJobStorage";
import { openReminders } from "./bestieListStorage";
import type { BestieListKind, BestieListState } from "./bestieListTypes";

/** Fixed RHS categories: reminders, todos, and jobs. */
export type BestieRhsKind = BestieListKind | "job" | "coffee" | "thread" | "scratch";

/** Omit empty counts the same way Projects overview does. */
export function presentBestieContextCount(
  value: number | undefined,
): number | undefined {
  return value != null && value > 0 ? value : undefined;
}

export function bestieCategoryTitle(kind: BestieRhsKind): string {
  if (kind === "reminder") return "Reminders";
  if (kind === "todo") return "To-dos";
  if (kind === "job") return "Jobs";
  if (kind === "coffee") return "Coffee";
  if (kind === "thread") return "Threads";
  return "Scratch";
}

/**
 * IdleAuxiliary (slide) is only for a drilled-in category’s item list.
 * Categories themselves live in the fixed RHS column — never the slide.
 * Returns null when the slide must stay closed (category home).
 */
export function bestieIdleAuxiliaryKind(
  activeKind: BestieRhsKind | null,
): BestieRhsKind | null {
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

export function presentBestieJobSchedule(schedule: {
  kind: string;
  dueAt?: number;
  everySeconds?: number;
  hour?: number;
  minute?: number;
  weekday?: number;
}): string {
  if (schedule.kind === "once" && schedule.dueAt != null) {
    return `Once · ${new Date(schedule.dueAt * 1000).toLocaleString()}`;
  }
  if (schedule.kind === "interval" && schedule.everySeconds != null) {
    const s = schedule.everySeconds;
    if (s % 3600 === 0) return `Every ${s / 3600}h`;
    if (s % 60 === 0) return `Every ${s / 60}m`;
    return `Every ${s}s`;
  }
  if (schedule.kind === "daily") {
    const h = schedule.hour ?? 0;
    const m = schedule.minute ?? 0;
    const pad = (n: number) => String(n).padStart(2, "0");
    return `Daily · ${pad(h)}:${pad(m)}`;
  }
  if (schedule.kind === "weekly") {
    const days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const day = days[(((schedule.weekday ?? 0) % 7) + 7) % 7] ?? "Sun";
    const pad = (n: number) => String(n).padStart(2, "0");
    return `Weekly · ${day} ${pad(schedule.hour ?? 0)}:${pad(schedule.minute ?? 0)}`;
  }
  return schedule.kind;
}

/** Soonest dueAt among open reminders (includes already-due). */
export function soonestOpenReminderDueAt(
  state: BestieListState,
): number | null {
  let soonest: number | null = null;
  for (const item of openReminders(state)) {
    if (item.dueAt == null) continue;
    if (soonest == null || item.dueAt < soonest) soonest = item.dueAt;
  }
  return soonest;
}

/** Soonest nextDueAt among enabled jobs (includes already-due). */
export function soonestEnabledJobDueAt(state: BestieJobState): number | null {
  let soonest: number | null = null;
  for (const job of enabledJobs(state)) {
    if (job.nextDueAt == null) continue;
    if (soonest == null || job.nextDueAt < soonest) soonest = job.nextDueAt;
  }
  return soonest;
}

