export const MAX_CONCURRENT_AUDIO_MEDIA_LOADS = 3;

type InternalTask = {
  controller: AbortController;
  reject: (reason?: unknown) => void;
  resolve: (value: unknown) => void;
  run: (signal: AbortSignal) => Promise<unknown>;
  settled: boolean;
  started: boolean;
};

export type AudioMediaLoadHandle<T> = {
  cancel: () => void;
  promise: Promise<T>;
};

const queuedTasks: InternalTask[] = [];
const activeTasks = new Set<InternalTask>();

function abortError(): DOMException {
  return new DOMException("Audio media load cancelled", "AbortError");
}

function settleQueuedCancellation(task: InternalTask): void {
  const index = queuedTasks.indexOf(task);
  if (index >= 0) queuedTasks.splice(index, 1);
  if (task.settled) return;
  task.settled = true;
  task.reject(abortError());
}

function pumpAudioMediaLoads(): void {
  while (
    activeTasks.size < MAX_CONCURRENT_AUDIO_MEDIA_LOADS &&
    queuedTasks.length > 0
  ) {
    const task = queuedTasks.shift();
    if (!task || task.settled) continue;
    if (task.controller.signal.aborted) {
      settleQueuedCancellation(task);
      continue;
    }

    task.started = true;
    activeTasks.add(task);
    void task
      .run(task.controller.signal)
      .then(task.resolve, task.reject)
      .finally(() => {
        task.settled = true;
        activeTasks.delete(task);
        pumpAudioMediaLoads();
      });
  }
}

/**
 * Run expensive audio transfer/decode work behind one shared hard cap.
 *
 * Cancelling a queued task removes it without starting it. Cancelling an
 * active task aborts its owned signal; the slot is released when the task's
 * abort-aware work has torn down.
 */
export function scheduleAudioMediaLoad<T>(
  run: (signal: AbortSignal) => Promise<T>,
): AudioMediaLoadHandle<T> {
  let resolvePromise: (value: T) => void = () => {};
  let rejectPromise: (reason?: unknown) => void = () => {};
  const promise = new Promise<T>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  const scheduledTask: InternalTask = {
    controller: new AbortController(),
    reject: rejectPromise,
    resolve: (value) => resolvePromise(value as T),
    run,
    settled: false,
    started: false,
  };
  queuedTasks.push(scheduledTask);
  pumpAudioMediaLoads();

  return {
    cancel: () => {
      if (scheduledTask.settled || scheduledTask.controller.signal.aborted) {
        return;
      }
      scheduledTask.controller.abort();
      if (!scheduledTask.started) {
        settleQueuedCancellation(scheduledTask);
        pumpAudioMediaLoads();
      }
    },
    promise,
  };
}

/** Test/diagnostic snapshot of the scheduler's exact ownership counts. */
export function getAudioMediaLoadSchedulerSnapshot(): {
  active: number;
  queued: number;
} {
  return { active: activeTasks.size, queued: queuedTasks.length };
}

/** Cancel all community-scoped audio work during a relay boundary switch. */
export function resetAudioMediaLoadScheduler(): void {
  for (const task of [...queuedTasks, ...activeTasks]) {
    if (task.settled || task.controller.signal.aborted) continue;
    task.controller.abort();
    if (!task.started) settleQueuedCancellation(task);
  }
  pumpAudioMediaLoads();
}
