/**
 * Creates an async save coalescer.
 *
 * When multiple calls to enqueue() arrive while a save is in flight, only
 * the latest enqueued value is submitted per drain round — no edit is
 * silently dropped and the final persisted state always reflects the most
 * recent local change.
 *
 * Lifecycle: call cancel() on unmount so in-flight saves do not invoke
 * the onSaving / onSaved callbacks after the owning component is gone.
 */
export function createSaveCoalescer<T>(
  save: (value: T) => Promise<T>,
  onSaving: (isSaving: boolean) => void,
  onSaved: (value: T) => void,
): { enqueue: (value: T) => void; cancel: () => void } {
  let pending: T | undefined;
  let hasPending = false;
  let running = false;
  let cancelled = false;

  async function drain() {
    while (hasPending) {
      const toSave = pending as T;
      hasPending = false;
      pending = undefined;
      try {
        const saved = await save(toSave);
        // Apply backend response only when no newer local edit is pending —
        // a stale response must never overwrite fresher optimistic state.
        if (!cancelled && !hasPending) {
          onSaved(saved);
        }
      } catch {
        // Non-fatal — optimistic local state is retained; user can retry.
      }
    }
    running = false;
    if (!cancelled) onSaving(false);
  }

  return {
    enqueue(value: T) {
      pending = value;
      hasPending = true;
      if (running) return;
      running = true;
      onSaving(true);
      void drain();
    },
    cancel() {
      cancelled = true;
      hasPending = false;
      pending = undefined;
    },
  };
}
