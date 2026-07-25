/**
 * Creates an async save coalescer.
 *
 * When multiple calls to enqueue() arrive while a save is in flight, only
 * the latest enqueued value is submitted per drain round — no edit is
 * silently dropped and the final persisted state always reflects the most
 * recent local change.
 *
 * Lifecycle: call cancel() on unmount so in-flight saves do not invoke
 * callbacks after the owning component is gone. Call flush() before leaving
 * a surface that must guarantee its latest optimistic value was persisted.
 */
export function createSaveCoalescer<T>(
  save: (value: T) => Promise<T>,
  onSaving: (isSaving: boolean) => void,
  onSaved: (value: T) => void,
): {
  enqueue: (value: T) => void;
  flush: () => Promise<void>;
  cancel: () => void;
} {
  let pending: T | undefined;
  let hasPending = false;
  let running = false;
  let cancelled = false;
  let finalError: unknown;
  let flushWaiters: Array<{
    resolve: () => void;
    reject: (error: unknown) => void;
  }> = [];

  function settleFlushWaiters() {
    const waiters = flushWaiters;
    flushWaiters = [];
    for (const waiter of waiters) {
      if (finalError === undefined) waiter.resolve();
      else waiter.reject(finalError);
    }
  }

  async function drain() {
    while (hasPending) {
      const toSave = pending as T;
      hasPending = false;
      pending = undefined;
      try {
        const saved = await save(toSave);
        finalError = undefined;
        // Apply backend response only when no newer local edit is pending —
        // a stale response must never overwrite fresher optimistic state.
        if (!cancelled && !hasPending) {
          onSaved(saved);
        }
      } catch (error) {
        finalError = error;
      }
    }
    running = false;
    if (!cancelled) {
      onSaving(false);
      settleFlushWaiters();
    }
  }

  return {
    enqueue(value: T) {
      pending = value;
      hasPending = true;
      if (running) return;
      running = true;
      finalError = undefined;
      onSaving(true);
      void drain();
    },
    flush() {
      if (!running) {
        return finalError === undefined
          ? Promise.resolve()
          : Promise.reject(finalError);
      }
      return new Promise<void>((resolve, reject) => {
        flushWaiters.push({ resolve, reject });
      });
    },
    cancel() {
      cancelled = true;
      hasPending = false;
      pending = undefined;
      const waiters = flushWaiters;
      flushWaiters = [];
      for (const waiter of waiters) {
        waiter.reject(new Error("Save cancelled"));
      }
    },
  };
}
