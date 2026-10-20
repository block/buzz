export type PresenceSubscriptionClose = () => Promise<void>;

export interface PresenceSubscriptionReconcilerOptions {
  open: (authors: string[]) => Promise<PresenceSubscriptionClose>;
  retryDelay?: (attempt: number) => number;
  setTimer?: (
    callback: () => void,
    delayMs: number,
  ) => ReturnType<typeof setTimeout>;
  clearTimer?: (timer: ReturnType<typeof setTimeout>) => void;
}

/**
 * Reconciles a changing author set onto one live relay subscription.
 *
 * A replacement opens before the prior subscription closes, so demand changes
 * never create a live-update gap. Opens that finish after demand changes are
 * closed without becoming current. A failed replacement leaves the last good
 * subscription serving its old author set while retrying with bounded backoff.
 */
export class PresenceSubscriptionReconciler {
  private readonly open: PresenceSubscriptionReconcilerOptions["open"];
  private readonly retryDelay: (attempt: number) => number;
  private readonly setTimer: NonNullable<
    PresenceSubscriptionReconcilerOptions["setTimer"]
  >;
  private readonly clearTimer: NonNullable<
    PresenceSubscriptionReconcilerOptions["clearTimer"]
  >;
  private desiredAuthors: string[] = [];
  private desiredKey = "";
  private current: { key: string; close: PresenceSubscriptionClose } | null =
    null;
  private running = false;
  private disposed = false;
  private retryAttempt = 0;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(options: PresenceSubscriptionReconcilerOptions) {
    this.open = options.open;
    this.retryDelay =
      options.retryDelay ??
      ((attempt) => Math.min(1000 * 2 ** attempt, 30_000));
    this.setTimer =
      options.setTimer ??
      ((callback, delayMs) => globalThis.setTimeout(callback, delayMs));
    this.clearTimer =
      options.clearTimer ?? ((timer) => globalThis.clearTimeout(timer));
  }

  setAuthors(authors: string[]) {
    if (this.disposed) return;
    const normalized = [
      ...new Set(authors.map((author) => author.toLowerCase())),
    ]
      .filter(Boolean)
      .sort();
    const key = normalized.join(",");
    if (key === this.desiredKey) return;

    this.desiredAuthors = normalized;
    this.desiredKey = key;
    this.retryAttempt = 0;
    this.cancelRetry();
    void this.reconcile();
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.cancelRetry();
    const current = this.current;
    this.current = null;
    if (current) void current.close().catch(() => {});
  }

  private currentKey() {
    return this.current?.key ?? "";
  }

  private cancelRetry() {
    if (this.retryTimer === null) return;
    this.clearTimer(this.retryTimer);
    this.retryTimer = null;
  }

  private scheduleRetry() {
    if (this.retryTimer !== null || this.disposed) return;
    const delay = this.retryDelay(this.retryAttempt);
    this.retryAttempt += 1;
    this.retryTimer = this.setTimer(() => {
      this.retryTimer = null;
      void this.reconcile();
    }, delay);
  }

  private async reconcile() {
    if (this.running || this.disposed) return;
    this.running = true;
    try {
      while (!this.disposed && this.currentKey() !== this.desiredKey) {
        const nextAuthors = this.desiredAuthors;
        const nextKey = this.desiredKey;

        if (nextAuthors.length === 0) {
          const previous = this.current;
          this.current = null;
          if (previous) await previous.close().catch(() => {});
          continue;
        }

        let nextClose: PresenceSubscriptionClose;
        try {
          nextClose = await this.open(nextAuthors);
        } catch {
          if (nextKey === this.desiredKey) this.scheduleRetry();
          return;
        }

        if (this.disposed || nextKey !== this.desiredKey) {
          await nextClose().catch(() => {});
          continue;
        }

        const previous = this.current;
        this.current = { key: nextKey, close: nextClose };
        this.retryAttempt = 0;
        if (previous) await previous.close().catch(() => {});
      }
    } finally {
      this.running = false;
      if (
        !this.disposed &&
        this.retryTimer === null &&
        this.currentKey() !== this.desiredKey
      ) {
        void this.reconcile();
      }
    }
  }
}
