import { dueReminders, openReminders, openTodos } from "./bestieListStorage";
import type { BestieListState } from "./bestieListTypes";
import type { BestieNudge } from "./bestieNudgeStore";

/** Default autonomous wake cadence (~5 minutes). */
export const BESTIE_WAKE_INTERVAL_MS = 5 * 60 * 1000;

/** Minimum delay before a due-reminder one-shot fire (avoid tight loops). */
export const BESTIE_DUE_WAKE_MIN_DELAY_MS = 1_000;

/** Cap for due-reminder one-shot so we still poll the interval path. */
export const BESTIE_DUE_WAKE_MAX_DELAY_MS = BESTIE_WAKE_INTERVAL_MS;

export type BestieWakeEvaluation = {
  nudge: BestieNudge | null;
  /** True when the Bestie agent should be ensured running. */
  shouldWakeAgent: boolean;
};

/** Next open reminder dueAt strictly after now, or null. */
export function nextOpenReminderDueAt(
  state: BestieListState,
  nowSeconds: number,
): number | null {
  let next: number | null = null;
  for (const item of openReminders(state)) {
    if (item.dueAt == null || item.dueAt <= nowSeconds) continue;
    if (next == null || item.dueAt < next) next = item.dueAt;
  }
  return next;
}

/**
 * Pure wake evaluator: due reminders + open todos → proactive nudge payload.
 * Distinct from normal DM unread — callers surface via footer/popover chrome.
 * Due reminders use reason "due-reminder" (popover auto-opens); todos-only use
 * "check-in" (badge + banner only).
 */
export function evaluateBestieWake(
  state: BestieListState,
  nowSeconds: number,
  options?: { previousNudgeId?: string | null },
): BestieWakeEvaluation {
  const due = dueReminders(state, nowSeconds);
  const todos = openTodos(state);
  if (due.length === 0 && todos.length === 0) {
    return { nudge: null, shouldWakeAgent: false };
  }

  const itemIds = [...due, ...todos].map((item) => item.id);
  const fingerprint = `${due
    .map((d) => d.id)
    .sort()
    .join(",")}|${todos
    .map((t) => t.id)
    .sort()
    .join(",")}`;
  const id = `wake:${fingerprint}`;
  if (options?.previousNudgeId === id) {
    // Same outstanding set — keep badge, don't re-fire wake storm.
    return {
      nudge: null,
      shouldWakeAgent: false,
    };
  }

  const parts: string[] = [];
  if (due.length > 0) {
    parts.push(
      due.length === 1
        ? `Reminder: ${due[0].text}`
        : `${due.length} reminders are due`,
    );
  }
  if (todos.length > 0) {
    parts.push(
      todos.length === 1
        ? `Open to-do: ${todos[0].text}`
        : `${todos.length} open to-dos`,
    );
  }

  const dueFocused = due.length > 0;
  return {
    nudge: {
      body: parts.join(" · "),
      createdAt: nowSeconds,
      id,
      itemIds,
      reason: dueFocused ? "due-reminder" : "check-in",
      title: dueFocused ? "Reminder due" : "Bestie check-in",
    },
    shouldWakeAgent: true,
  };
}

export type BestieWakeSchedulerHandles = {
  stop: () => void;
  /** Fire one evaluation immediately (also used by tests). */
  tick: () => BestieWakeEvaluation;
};

/**
 * Interval wake loop + one-shot timer for the next due reminder.
 * Call `stop()` on unmount.
 * `onWakeAgent` should be best-effort (ensure agent running).
 */
export function startBestieWakeScheduler(options: {
  getListState: () => BestieListState;
  getPreviousNudgeId: () => string | null;
  intervalMs?: number;
  nowSeconds?: () => number;
  onNudge: (nudge: BestieNudge) => void;
  onWakeAgent: () => void;
}): BestieWakeSchedulerHandles {
  let lastEmittedId: string | null = options.getPreviousNudgeId();
  const intervalMs = options.intervalMs ?? BESTIE_WAKE_INTERVAL_MS;
  let dueTimer: number | null = null;

  const clearDueTimer = () => {
    if (dueTimer != null) {
      window.clearTimeout(dueTimer);
      dueTimer = null;
    }
  };

  const tick = (): BestieWakeEvaluation => {
    const now = options.nowSeconds?.() ?? Math.floor(Date.now() / 1000);
    const listState = options.getListState();
    const evaluation = evaluateBestieWake(listState, now, {
      previousNudgeId: lastEmittedId,
    });
    if (evaluation.nudge) {
      lastEmittedId = evaluation.nudge.id;
      options.onNudge(evaluation.nudge);
    }
    if (evaluation.shouldWakeAgent) {
      try {
        options.onWakeAgent();
      } catch {
        // Wake is best-effort; nudge UI still surfaces.
      }
    }

    clearDueTimer();
    const nextDue = nextOpenReminderDueAt(listState, now);
    if (nextDue != null) {
      const delayMs = Math.min(
        BESTIE_DUE_WAKE_MAX_DELAY_MS,
        Math.max(BESTIE_DUE_WAKE_MIN_DELAY_MS, (nextDue - now) * 1000),
      );
      dueTimer = window.setTimeout(() => {
        dueTimer = null;
        tick();
      }, delayMs);
    }
    return evaluation;
  };

  const timer = window.setInterval(tick, intervalMs);
  // Immediate first pass so a already-due reminder surfaces without waiting 5m.
  tick();
  return {
    stop: () => {
      window.clearInterval(timer);
      clearDueTimer();
    },
    tick,
  };
}
