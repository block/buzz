import type { BestieJobSchedule } from "./bestieJobTypes";

/** Minimum interval between recurring job fires (avoid tight loops). */
export const BESTIE_JOB_MIN_INTERVAL_SECONDS = 60;

/**
 * Compute the next due unix seconds for a schedule at or after `fromSeconds`.
 * Returns null when the schedule cannot produce a future time (invalid once).
 */
export function computeBestieJobNextDueAt(
  schedule: BestieJobSchedule,
  fromSeconds: number,
  options?: { afterRun?: boolean },
): number | null {
  const afterRun = options?.afterRun === true;
  if (schedule.kind === "once") {
    if (afterRun) return null;
    return schedule.dueAt >= fromSeconds ? schedule.dueAt : null;
  }
  if (schedule.kind === "interval") {
    const every = Math.max(
      BESTIE_JOB_MIN_INTERVAL_SECONDS,
      Math.floor(schedule.everySeconds),
    );
    if (afterRun) return fromSeconds + every;
    // Align to next boundary from epoch-ish steps is unnecessary; first due is now+every
    // only when creating without an explicit first fire — callers set nextDueAt on create.
    return fromSeconds + every;
  }
  if (schedule.kind === "daily") {
    return nextDailyDueAt(
      schedule.hour,
      schedule.minute,
      fromSeconds,
      afterRun,
    );
  }
  return nextWeeklyDueAt(
    schedule.weekday,
    schedule.hour,
    schedule.minute,
    fromSeconds,
    afterRun,
  );
}

function nextDailyDueAt(
  hour: number,
  minute: number,
  fromSeconds: number,
  afterRun: boolean,
): number {
  const base = new Date(fromSeconds * 1000);
  const candidate = new Date(base);
  candidate.setSeconds(0, 0);
  candidate.setHours(clampHour(hour), clampMinute(minute), 0, 0);
  let due = Math.floor(candidate.getTime() / 1000);
  if (afterRun || due <= fromSeconds) {
    candidate.setDate(candidate.getDate() + 1);
    due = Math.floor(candidate.getTime() / 1000);
  }
  return due;
}

function nextWeeklyDueAt(
  weekday: number,
  hour: number,
  minute: number,
  fromSeconds: number,
  afterRun: boolean,
): number {
  const targetDow = ((Math.floor(weekday) % 7) + 7) % 7;
  const base = new Date(fromSeconds * 1000);
  const candidate = new Date(base);
  candidate.setSeconds(0, 0);
  candidate.setHours(clampHour(hour), clampMinute(minute), 0, 0);
  const currentDow = candidate.getDay();
  let addDays = (targetDow - currentDow + 7) % 7;
  if (addDays === 0) {
    const due = Math.floor(candidate.getTime() / 1000);
    if (afterRun || due <= fromSeconds) addDays = 7;
  }
  if (addDays > 0) candidate.setDate(candidate.getDate() + addDays);
  return Math.floor(candidate.getTime() / 1000);
}

function clampHour(hour: number): number {
  if (!Number.isFinite(hour)) return 9;
  return Math.min(23, Math.max(0, Math.floor(hour)));
}

function clampMinute(minute: number): number {
  if (!Number.isFinite(minute)) return 0;
  return Math.min(59, Math.max(0, Math.floor(minute)));
}

export function bestieJobFireSlotId(jobId: string, dueAt: number): string {
  return `${jobId}@${dueAt}`;
}

/** Format the outbound turn Bestie receives when a job fires. */
export function formatBestieJobRunPrompt(
  title: string,
  prompt: string,
): string {
  const cleanTitle = title.trim() || "Job";
  const cleanPrompt = prompt.trim();
  return `[Bestie job: ${cleanTitle}]\n\n${cleanPrompt}`;
}

export const BESTIE_JOB_RUN_MARKER = "[Bestie job:";
