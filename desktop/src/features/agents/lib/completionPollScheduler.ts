export type TimerHandle = ReturnType<typeof setTimeout>;

type CompletionPollSchedulerOptions = {
  poll: () => Promise<void>;
  delayMs: number;
  scheduleTimer?: (callback: () => void, delayMs: number) => TimerHandle;
  cancelTimer?: (handle: TimerHandle) => void;
};

export type CompletionPollScheduler = {
  /** Start a poll unless one is already running. Concurrent callers share it. */
  trigger: () => Promise<void>;
  /** Cancel future polls. An already-running poll is allowed to settle. */
  stop: () => void;
};

/**
 * Run one poll immediately, then wait `delayMs` after each completed poll
 * before starting the next one. This avoids the backlog created by intervals
 * when native work takes longer than the nominal polling cadence.
 */
export function createCompletionPollScheduler({
  poll,
  delayMs,
  scheduleTimer = setTimeout,
  cancelTimer = clearTimeout,
}: CompletionPollSchedulerOptions): CompletionPollScheduler {
  let stopped = false;
  let timer: TimerHandle | null = null;
  let inFlight: Promise<void> | null = null;

  const trigger = (): Promise<void> => {
    if (stopped) return Promise.resolve();
    if (inFlight) return inFlight;
    if (timer !== null) {
      cancelTimer(timer);
      timer = null;
    }

    const running = Promise.resolve()
      .then(poll)
      .finally(() => {
        if (inFlight === running) inFlight = null;
        if (stopped) return;
        timer = scheduleTimer(() => {
          timer = null;
          void trigger().catch(() => {});
        }, delayMs);
      });
    inFlight = running;
    return running;
  };

  const stop = (): void => {
    stopped = true;
    if (timer !== null) {
      cancelTimer(timer);
      timer = null;
    }
  };

  void trigger().catch(() => {});
  return { trigger, stop };
}
