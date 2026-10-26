const MAX_TIMERS = 32;
const WORKER_SOURCE = `
const timers = new Map();
self.onmessage = ({ data }) => {
  if (data.cancel !== undefined) {
    clearTimeout(timers.get(data.cancel));
    timers.delete(data.cancel);
    return;
  }
  if (timers.size >= ${MAX_TIMERS}) throw new Error("Too many pending huddle timers");
  const remaining = Math.max(0, data.deadline - performance.timeOrigin - performance.now());
  const timer = setTimeout(() => {
    timers.delete(data.id);
    self.postMessage(data.id);
  }, remaining);
  timers.set(data.id, timer);
};`;

/** A session-owned clock that keeps opted-in media moving in a hidden webview. */
export class HuddleMediaClock {
  private readonly worker: Worker;
  private readonly url: string;
  private readonly pending = new Map<number, () => void>();
  private nextId = 0;
  private disposed = false;

  constructor(onFailure: (error: Error) => void) {
    this.url = URL.createObjectURL(
      new Blob([WORKER_SOURCE], { type: "text/javascript" }),
    );
    try {
      this.worker = new Worker(this.url);
    } catch (error) {
      URL.revokeObjectURL(this.url);
      throw error;
    }
    this.worker.onmessage = ({ data }: MessageEvent<unknown>) => {
      if (this.disposed || typeof data !== "number") return;
      const callback = this.pending.get(data);
      this.pending.delete(data);
      callback?.();
    };
    this.worker.onerror = (event) => {
      event.preventDefault();
      if (this.disposed) return;
      this.dispose();
      onFailure(new Error(event.message || "Huddle media clock failed"));
    };
  }

  /** Schedule one bounded wake-up; frame/IPC backpressure owns the next one. */
  schedule(callback: () => void, delay: number): number {
    if (this.disposed) throw new Error("Huddle media clock is closed");
    if (
      this.pending.size >= MAX_TIMERS ||
      !Number.isFinite(delay) ||
      delay < 0 ||
      delay > 60_000 ||
      this.nextId >= Number.MAX_SAFE_INTEGER
    ) {
      throw new Error("Invalid or excessive huddle media timers");
    }
    const id = ++this.nextId;
    this.pending.set(id, callback);
    try {
      this.worker.postMessage({
        id,
        delay,
        // Match a browser timer's minimum yield and account for the worker hop.
        deadline:
          (performance.timeOrigin ?? 0) +
          performance.now() +
          Math.max(4, delay),
      });
    } catch (error) {
      this.pending.delete(id);
      throw error;
    }
    return id;
  }

  /** A cancelled or late worker response cannot revive a retired producer. */
  unschedule(timer: unknown) {
    if (
      this.disposed ||
      typeof timer !== "number" ||
      !this.pending.delete(timer)
    )
      return;
    this.worker.postMessage({ cancel: timer });
  }

  /** Release all callbacks, the worker, and its blob on every stop path. */
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.pending.clear();
    this.worker.onmessage = null;
    this.worker.onerror = null;
    this.worker.terminate();
    URL.revokeObjectURL(this.url);
  }
}
