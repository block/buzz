/** Error returned to a caller when a queued or running local run is cancelled. */
export class LocalRunCancelledError extends Error {
  constructor(taskId: string) {
    super(`Local run "${taskId}" was cancelled.`);
    this.name = "LocalRunCancelledError";
  }
}

/** Maximum UTF-8 size of a local-run task identity retained by the scheduler. */
export const MAX_LOCAL_RUN_TASK_ID_BYTES = 256;

const TASK_ID_UTF8_ENCODER = new TextEncoder();

function hasControlCharacters(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}

function isValidTaskId(taskId: unknown): taskId is string {
  return (
    typeof taskId === "string" &&
    taskId.length > 0 &&
    taskId.length <= MAX_LOCAL_RUN_TASK_ID_BYTES &&
    taskId === taskId.trim() &&
    !hasControlCharacters(taskId) &&
    TASK_ID_UTF8_ENCODER.encode(taskId).byteLength <=
      MAX_LOCAL_RUN_TASK_ID_BYTES
  );
}

export type LocalRunTask<T> = (signal: AbortSignal) => PromiseLike<T> | T;

export type ScheduledLocalRun<T> = {
  readonly taskId: string;
  readonly result: Promise<T>;
  cancel(): boolean;
};

type QueuedRun<T> = {
  readonly abortController: AbortController;
  readonly reject: (reason: unknown) => void;
  readonly resolve: (value: T | PromiseLike<T>) => void;
  readonly task: LocalRunTask<T>;
  readonly taskId: string;
  cancelled: boolean;
  running: boolean;
  settled: boolean;
};

export type LocalRunSchedulerOptions = {
  readonly capacity?: number;
};

/**
 * Dependency-light FIFO scheduler for bounded local-model execution.
 *
 * Running cancellation aborts the task signal but retains its capacity until
 * the underlying task settles, preventing an abort-ignoring task from causing
 * accidental over-subscription.
 */
export class LocalRunScheduler {
  readonly #capacity: number;
  readonly #liveTaskIds = new Set<string>();
  readonly #queue: QueuedRun<unknown>[] = [];
  #active = 0;
  #draining = false;

  constructor(options: LocalRunSchedulerOptions = {}) {
    const capacity = options.capacity ?? 1;
    if (!Number.isInteger(capacity) || capacity < 1 || capacity > 2) {
      throw new RangeError("Local run capacity must be either 1 or 2.");
    }
    this.#capacity = capacity;
  }

  enqueue<T>(taskId: string, task: LocalRunTask<T>): ScheduledLocalRun<T> {
    if (!isValidTaskId(taskId) || typeof task !== "function") {
      throw new TypeError(
        `A local run requires a trimmed, control-free task ID of at most ${MAX_LOCAL_RUN_TASK_ID_BYTES} UTF-8 bytes and a task.`,
      );
    }
    if (this.#liveTaskIds.has(taskId)) {
      throw new Error(`Local run "${taskId}" is already queued or running.`);
    }

    let resolveResult: (value: T | PromiseLike<T>) => void = () => {};
    let rejectResult: (reason: unknown) => void = () => {};
    const result = new Promise<T>((resolve, reject) => {
      resolveResult = resolve;
      rejectResult = reject;
    });
    // The returned promise still rejects for callers that await it. This
    // internal observer prevents a deliberately fire-and-forget run from
    // becoming a process-level unhandled rejection.
    void result.catch(() => {});
    const run: QueuedRun<T> = {
      abortController: new AbortController(),
      reject: rejectResult,
      resolve: resolveResult,
      task,
      taskId,
      cancelled: false,
      running: false,
      settled: false,
    };
    this.#liveTaskIds.add(taskId);
    this.#queue.push(run as QueuedRun<unknown>);
    this.#drain();

    return Object.freeze({
      taskId,
      result,
      cancel: () => this.#cancel(run as QueuedRun<unknown>),
    });
  }

  #cancel(run: QueuedRun<unknown>): boolean {
    if (run.cancelled || run.settled) return false;
    run.cancelled = true;
    run.abortController.abort();
    run.settled = true;
    run.reject(new LocalRunCancelledError(run.taskId));
    if (!run.running) {
      this.#liveTaskIds.delete(run.taskId);
      this.#drain();
    }
    return true;
  }

  #drain(): void {
    if (this.#draining) return;
    this.#draining = true;
    try {
      while (this.#active < this.#capacity) {
        const run = this.#queue.shift();
        if (!run) break;
        if (run.cancelled) continue;
        this.#start(run);
      }
    } finally {
      this.#draining = false;
    }
  }

  #start(run: QueuedRun<unknown>): void {
    run.running = true;
    this.#active += 1;
    Promise.resolve()
      .then(() => run.task(run.abortController.signal))
      .then(
        (value) => this.#finish(run, { ok: true, value }),
        (error: unknown) => this.#finish(run, { error, ok: false }),
      );
  }

  #finish(
    run: QueuedRun<unknown>,
    outcome:
      | { readonly ok: true; readonly value: unknown }
      | { readonly error: unknown; readonly ok: false },
  ): void {
    run.running = false;
    this.#active -= 1;
    this.#liveTaskIds.delete(run.taskId);
    this.#drain();
    if (run.settled) return;
    run.settled = true;
    if (outcome.ok) {
      run.resolve(outcome.value);
    } else {
      run.reject(outcome.error);
    }
  }
}
