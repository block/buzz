import { rateLimitRemainingMs } from "./relayRateLimitGate";

// Deliberately below the relay's default total WS allowance: other request
// families and publications share that budget. This is pacing, not a quota guarantee.
const LIVE_REQ_INTERVAL_MS = 250;
type PendingReq = {
  current: () => boolean;
  priority: () => number;
  send: () => Promise<void>;
  resolve: () => void;
  reject: (error: unknown) => void;
  promise: Promise<void>;
};

/** Session-owned cold/live-retry drain; entries are bounded by owned subscriptions. */
export class RelayLiveReqDrain {
  private pending = new Map<string, PendingReq>();
  private timer: number | undefined;
  private nextAt = 0;

  run(
    id: string,
    current: () => boolean,
    priority: () => number,
    send: () => Promise<void>,
  ): Promise<void> {
    const existing = this.pending.get(id);
    if (existing) return existing.promise;
    let resolve = () => {};
    let reject = (_error: unknown) => {};
    const promise = new Promise<void>((ok, fail) => {
      resolve = ok;
      reject = fail;
    });
    this.pending.set(id, { current, priority, send, promise, resolve, reject });
    this.drain();
    return promise;
  }

  cancel(id: string) {
    const entry = this.pending.get(id);
    this.pending.delete(id);
    entry?.resolve();
    if (this.pending.size === 0) {
      window.clearTimeout(this.timer);
      this.timer = undefined;
    }
  }

  reset() {
    for (const id of this.pending.keys()) this.cancel(id);
    this.nextAt = 0;
  }

  private drain = () => {
    window.clearTimeout(this.timer);
    this.timer = undefined;
    for (const [id, entry] of this.pending) {
      if (!entry.current()) this.cancel(id);
    }
    if (this.pending.size === 0) return;
    const delay = Math.max(this.nextAt - Date.now(), rateLimitRemainingMs());
    if (delay > 0) {
      this.timer = window.setTimeout(this.drain, delay);
      return;
    }
    const entries = [...this.pending];
    const selected = entries.sort(
      (a, b) => a[1].priority() - b[1].priority(),
    )[0];
    if (!selected) return;
    const [id, entry] = selected;
    this.pending.delete(id);
    this.nextAt = Date.now() + LIVE_REQ_INTERVAL_MS;
    // send checks ownership again immediately before IPC. Do not await its
    // outcome here: publications and independent subscriptions must keep moving.
    void entry.send().then(entry.resolve, entry.reject);
    if (this.pending.size > 0) {
      this.timer = window.setTimeout(this.drain, LIVE_REQ_INTERVAL_MS);
    }
  };
}
