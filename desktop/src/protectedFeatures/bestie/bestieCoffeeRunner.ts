import {
  BESTIE_DUE_WAKE_MAX_DELAY_MS,
  BESTIE_DUE_WAKE_MIN_DELAY_MS,
  BESTIE_WAKE_INTERVAL_MS,
} from "./bestieWakeScheduler";
import {
  coffeeScheduleGate,
  nextCoffeeDueAt,
} from "./bestieCoffeeSchedule";
import type { BestieCoffeeState } from "./bestieCoffeeTypes";

export type BestieCoffeeRunnerHandles = {
  stop: () => void;
  tick: () => boolean;
};

/**
 * Interval + one-shot timer for the morning Coffee schedule.
 * `onDueCoffee` fires at most once per local day, only inside the 08:00
 * window, when the agent is online. Late remount/presence must not catch up.
 * Caller must begin the pending run (idempotent lock) before sending the turn.
 */
export function startBestieCoffeeRunner(options: {
  getCoffeeState: () => BestieCoffeeState;
  getIsAgentOnline: () => boolean;
  intervalMs?: number;
  nowSeconds?: () => number;
  onDueCoffee: () => void;
  /** Past 08:00 window, day unclaimed — claim skip, do not fire. */
  onMissedWindow?: (nowSeconds: number) => void;
}): BestieCoffeeRunnerHandles {
  const intervalMs = options.intervalMs ?? BESTIE_WAKE_INTERVAL_MS;
  let dueTimer: number | null = null;

  const clearDueTimer = () => {
    if (dueTimer != null) {
      window.clearTimeout(dueTimer);
      dueTimer = null;
    }
  };

  const scheduleNext = (now: number, state: BestieCoffeeState) => {
    clearDueTimer();
    const nextDue = nextCoffeeDueAt(state.prefs, now);
    const delayMs = Math.min(
      BESTIE_DUE_WAKE_MAX_DELAY_MS,
      Math.max(BESTIE_DUE_WAKE_MIN_DELAY_MS, (nextDue - now) * 1000),
    );
    dueTimer = window.setTimeout(() => {
      dueTimer = null;
      tick();
    }, delayMs);
  };

  const tick = (): boolean => {
    const now = options.nowSeconds?.() ?? Math.floor(Date.now() / 1000);
    const state = options.getCoffeeState();
    const gate = coffeeScheduleGate({
      isAgentOnline: options.getIsAgentOnline(),
      nowSeconds: now,
      state,
    });
    let fired = false;
    if (gate.ok) {
      try {
        options.onDueCoffee();
        fired = true;
      } catch {
        // Fire is best-effort.
      }
    } else if (gate.reason === "missed-window" && options.onMissedWindow) {
      // Claim the day without posting — blocks late remount catch-up.
      try {
        options.onMissedWindow(now);
      } catch {
        // Best-effort claim.
      }
    }
    scheduleNext(now, options.getCoffeeState());
    return fired;
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
