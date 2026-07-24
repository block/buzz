/**
 * Reconnect controller for the relay transport.
 *
 * Implements a three-phase strategy:
 *
 * 1. **Fast path** — attempt `preconnect()` with a short timeout. Succeeds
 *    immediately for transient network blips where the client-side VPN is
 *    already healthy; no configured hook fires and no browser action is taken.
 *
 * 2. **Escalation** (only when fast path fails AND a hook is configured) —
 *    invoke the build-time configured transport-recovery hook. The hook
 *    returns quickly; any browser-based reconnect flow it triggers runs
 *    asynchronously.
 *
 * 3. **Wait for background reconnect** — observe the session's exponential-
 *    backoff reconnect loop. Success is declared when its connection-state
 *    emitter reports connected; the backstop is a UI ceiling, not a delay.
 *
 * The controller is a module-level singleton so all hook instances in the
 * same app share a single in-flight state. A cancellation token is bumped on
 * every new attempt and checked after every `await` and inside every async
 * continuation, preventing state mutations from a superseded attempt.
 *
 * Dependencies (`preconnect`, `hookConfigured`, `runHook`,
 * `subscribeToConnectionState`) are injected, making the controller
 * testable without React or Tauri.
 */

/** Timer durations used by the reconnect controller. */
export type ReconnectTimingPolicy = {
  /** Short deadline for the optimistic fast-path attempt. */
  fastPathTimeoutMs: number;
  /** Maximum total time to wait before returning control to the UI. */
  backstopMs: number;
};

/** Current production reconnect timings. */
export const DEFAULT_RECONNECT_TIMING_POLICY: ReconnectTimingPolicy = {
  fastPathTimeoutMs: 11_000,
  backstopMs: 120_000,
};

export type ReconnectState = {
  isPending: boolean;
  isWaitingOnReconnectHook: boolean;
};

type Listener = (state: ReconnectState) => void;

export type ReconnectDeps = {
  preconnect: () => Promise<void>;
  hookConfigured: () => Promise<boolean>;
  runHook: () => Promise<void>;
  subscribeToConnectionState: (listener: (state: string) => void) => () => void;
  onSuccess: () => void;
  onBackstop: () => void;
  setTimeout: (fn: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
};

function withDeadline<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
  setTimeoutFn: (fn: () => void, ms: number) => number,
  clearTimeoutFn: (id: number) => void,
): Promise<T> {
  let id: number | null = null;
  const deadline = new Promise<never>((_, reject) => {
    id = setTimeoutFn(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  return Promise.race([promise, deadline]).finally(() => {
    if (id !== null) clearTimeoutFn(id);
  });
}

export class RelayReconnectController {
  private readonly timingPolicy: ReconnectTimingPolicy;

  constructor(
    timingPolicy: ReconnectTimingPolicy = DEFAULT_RECONNECT_TIMING_POLICY,
  ) {
    this.timingPolicy = timingPolicy;
  }

  private state: ReconnectState = {
    isPending: false,
    isWaitingOnReconnectHook: false,
  };
  private listeners = new Set<Listener>();
  // Cancellation token: bumped at the start of each attempt AND on cancel.
  // All async continuations capture the token at their creation point and
  // bail if it has since been superseded.
  private attemptToken = 0;
  // Active timer and subscription for the current attempt. The timer-clear
  // function is stored from the deps of the most recent start() call so that
  // cancel() and teardown do not need the caller to supply deps again.
  private backstopId: number | null = null;
  private unsubscribeConnectionState: (() => void) | null = null;
  private clearTimeoutFn: ((id: number) => void) | null = null;

  /** Subscribe to state changes. Fires immediately with the current state. */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    listener(this.state);
    return () => {
      this.listeners.delete(listener);
      // Cancel the in-flight attempt when the last subscriber leaves — no UI
      // is watching the result, so letting backstop callbacks fire
      // (possibly mutating state or invoking onSuccess/onBackstop into a
      // stale closure) would be a resource and correctness leak.
      if (this.listeners.size === 0) {
        this.cancel();
      }
    };
  }

  /** Current state snapshot — synchronous read. */
  getState(): ReconnectState {
    return this.state;
  }

  /**
   * Start a reconnect attempt. Returns false if one is already in flight.
   * On phase-3 entry returns false immediately; success/failure are
   * delivered asynchronously via state updates.
   */
  async start(deps: ReconnectDeps): Promise<boolean> {
    if (this.state.isPending) return false;

    // Store timer-clear fns so cancel() can clear timers without caller deps.
    this.clearTimeoutFn = deps.clearTimeout;

    // Bump the cancellation token before any await.
    const token = ++this.attemptToken;
    this.cancelTimers();
    this.setState({ isPending: true, isWaitingOnReconnectHook: false });

    const cancelled = () => token !== this.attemptToken;

    // ── Phase 1: fast path ───────────────────────────────────────────────────
    try {
      await withDeadline(
        deps.preconnect(),
        this.timingPolicy.fastPathTimeoutMs,
        "fast-path",
        deps.setTimeout,
        deps.clearTimeout,
      );
      if (cancelled()) return false;
      this.finish(deps.onSuccess, true);
      return true;
    } catch {
      if (cancelled()) return false;
      // Fast path failed — continue to escalation or background reconnect.
    }

    // ── Phase 2: escalation (hook-configured builds only) ───────────────────
    let hookConfigured = false;
    try {
      hookConfigured = await deps.hookConfigured();
    } catch (err) {
      console.warn(
        "[RelayReconnectController] hook configured check failed:",
        err,
      );
    }
    if (cancelled()) return false;

    if (hookConfigured) {
      try {
        await deps.runHook();
      } catch (err) {
        // Non-fatal — hook failure means the browser-based reconnect flow may
        // not have opened, but the session still retries in the background.
        console.warn(
          "[RelayReconnectController] transport recovery hook failed:",
          err,
        );
      }
      if (cancelled()) return false;
      this.setState({ isPending: true, isWaitingOnReconnectHook: true });
    }

    // ── Phase 3: wait for background reconnect ──────────────────────────────
    // The fast-path preconnect arms the session's exponential-backoff loop.
    // Observe it rather than issuing fixed-cadence attempts that consume its
    // pending retry timer.
    let resolved = false;

    // Capture onSuccess/onBackstop at phase-3 entry so that cancel() (which
    // bumps the token) never races with a late finish() invocation: the token
    // check in onConnected/backstop always wins before the callback is called.
    const { onSuccess, onBackstop } = deps;

    const onConnected = () => {
      if (resolved || cancelled()) return;
      resolved = true;
      this.finish(onSuccess, true);
    };

    // Subscribe FIRST. subscribeToConnectionState may invoke the listener
    // synchronously with the current state (documented on the production
    // implementation). If that sync emission fires onConnected(), `resolved`
    // is set to true inside the call — but the return value (the cleanup
    // handle) hasn't been assigned to `this.unsubscribeConnectionState` yet,
    // so cancelTimers() inside finish() cannot reach it. The `if (resolved)`
    // block below handles that late-assignment cleanup.
    this.unsubscribeConnectionState = deps.subscribeToConnectionState(
      (state) => {
        if (state === "connected") onConnected();
      },
    );

    // If the sync emission already finished the attempt, don't install timers
    // that would live for the app lifetime. The cleanup handle was assigned
    // after cancelTimers() ran inside finish(), so unsubscribe it now.
    if (resolved) {
      if (this.unsubscribeConnectionState !== null) {
        this.unsubscribeConnectionState();
        this.unsubscribeConnectionState = null;
      }
      return false;
    }

    this.backstopId = deps.setTimeout(() => {
      if (resolved || cancelled()) return;
      resolved = true;
      // The session's exponential-backoff reconnect loop remains armed.
      onBackstop();
      this.finish(() => {}, false);
    }, this.timingPolicy.backstopMs);

    // Return false now; async success is delivered via state updates.
    return false;
  }

  /**
   * Cancel the active attempt. Safe to call without a running attempt.
   * Bumps the cancellation token so any in-flight async continuations
   * (including a pending fast-path await) become no-ops.
   */
  cancel(): void {
    // Invalidate the token FIRST so any pending await that resolves
    // immediately after this call still sees a cancelled state.
    ++this.attemptToken;
    this.cancelTimers();
    if (this.state.isPending) {
      this.setState({ isPending: false, isWaitingOnReconnectHook: false });
    }
  }

  private finish(onSuccess: () => void, success: boolean): void {
    this.cancelTimers();
    this.setState({ isPending: false, isWaitingOnReconnectHook: false });
    if (success) onSuccess();
  }

  private cancelTimers(): void {
    if (this.backstopId !== null && this.clearTimeoutFn !== null) {
      this.clearTimeoutFn(this.backstopId);
      this.backstopId = null;
    }
    if (this.unsubscribeConnectionState !== null) {
      this.unsubscribeConnectionState();
      this.unsubscribeConnectionState = null;
    }
  }

  private setState(next: ReconnectState): void {
    this.state = next;
    for (const listener of this.listeners) {
      listener(next);
    }
  }
}

/** Module-level singleton — shared by all `useReconnectRelay` instances. */
export const relayReconnectController = new RelayReconnectController();
