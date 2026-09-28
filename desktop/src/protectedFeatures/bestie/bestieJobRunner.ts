import {
  BESTIE_DUE_WAKE_MAX_DELAY_MS,
  BESTIE_DUE_WAKE_MIN_DELAY_MS,
  BESTIE_WAKE_INTERVAL_MS,
} from "./bestieWakeScheduler";
import { dueBestieJobs, nextBestieJobDueAt } from "./bestieJobStorage";
import type { BestieJob, BestieJobState } from "./bestieJobTypes";

export type BestieJobRunnerHandles = {
  stop: () => void;
  tick: () => BestieJob[];
};

/**
 * Interval + one-shot timer for due Bestie jobs.
 * `onDueJobs` receives jobs that are due and not yet fired for their slot;
 * the caller must mark them fired (idempotent) before/while sending the turn.
 */
export function startBestieJobRunner(options: {
  getJobState: () => BestieJobState;
  intervalMs?: number;
  nowSeconds?: () => number;
  onDueJobs: (jobs: BestieJob[]) => void;
}): BestieJobRunnerHandles {
  const intervalMs = options.intervalMs ?? BESTIE_WAKE_INTERVAL_MS;
  let dueTimer: number | null = null;

  const clearDueTimer = () => {
    if (dueTimer != null) {
      window.clearTimeout(dueTimer);
      dueTimer = null;
    }
  };

  const tick = (): BestieJob[] => {
    const now = options.nowSeconds?.() ?? Math.floor(Date.now() / 1000);
    const state = options.getJobState();
    const due = dueBestieJobs(state, now);
    if (due.length > 0) {
      try {
        options.onDueJobs(due);
      } catch {
        // Fire is best-effort.
      }
    }
    clearDueTimer();
    // Re-read — caller may have advanced nextDueAt.
    const nextDue = nextBestieJobDueAt(options.getJobState(), now);
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
    return due;
  };

  const timer = window.setInterval(tick, intervalMs);
  tick();
  return {
    stop: () => {
      window.clearInterval(timer);
      clearDueTimer();
    },
    tick,
  };
}
