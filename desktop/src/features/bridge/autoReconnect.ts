// Keeps a dropped Direct connection coming back on its own: exponential backoff with ±25% jitter (1 s → 30 s cap),
// retried forever while the page is visible, and immediately when the app resumes (visible / pageshow / focus / online).
// Hidden pages wait for the resume signal instead of signing in the background.
import { isPageVisible, onResume } from "@/shared/lib/resumeSignals";

export const RECONNECT_BASE_MS = 1_000;
export const RECONNECT_CAP_MS = 30_000;

export function backoffDelay(
  failures: number,
  rand: () => number = Math.random,
): number {
  const raw = Math.min(
    RECONNECT_CAP_MS,
    RECONNECT_BASE_MS * 2 ** Math.max(0, failures),
  );
  return Math.round(Math.min(RECONNECT_CAP_MS, raw * (0.75 + rand() * 0.5)));
}

export class AutoReconnector {
  private failures = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  private busy = false;
  /** resume subscription, installed on first outage */
  offResume: (() => void) | null = null;
  /** When the current outage started (ms), 0 while healthy. */
  failingSince = 0;

  private attempt: () => Promise<boolean>;
  private onChange: () => void;
  constructor(
    attempt: () => Promise<boolean>,
    onChange: () => void = () => {},
  ) {
    this.attempt = attempt;
    this.onChange = onChange;
  }

  get active() {
    return this.running;
  }

  /** The connection dropped: start retrying (no-op if already retrying). */
  start() {
    if (this.running) return;
    this.running = true;
    this.failures = 0;
    this.failingSince = Date.now();
    this.offResume ??= onResume(() => this.now());
    this.onChange();
    this.schedule(300);
  }

  /** Healthy again (or the user turned the feature off): stop retrying. */
  stop() {
    this.running = false;
    this.failingSince = 0;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.onChange();
  }

  /** Try right now (resume signal or the optional "Reconnect now" button). */
  now() {
    if (!this.running) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    void this.run();
  }

  private schedule(ms: number) {
    if (!this.running) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.run();
    }, ms);
  }

  private async run() {
    if (!this.running || this.busy) return;
    if (!isPageVisible()) return; // resumes via onResume
    this.busy = true;
    let ok = false;
    try {
      ok = await this.attempt();
    } catch {
      ok = false;
    }
    this.busy = false;
    if (!this.running) return;
    if (ok) return this.stop();
    this.schedule(backoffDelay(this.failures++));
    this.onChange();
  }
}
