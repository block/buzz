/**
 * Idle auto-reload backstop — the pure decision core.
 *
 * A long-lived renderer session accumulates heap and timer churn until GC
 * pauses queue behind input (the multi-GB, multi-hour sessions that motivate
 * this feature). The Cmd+R clean-teardown reload discards that state without
 * touching the native engine or its child agents. This backstop performs the
 * same reload automatically, but ONLY when the user is provably away and the
 * session is old enough to have accumulated the churn — never during use.
 *
 * SAFETY: the reload navigates the webview (`location.reload()`), so a wrong
 * "fire" interrupts the user or drops in-flight work. Every gate below is
 * load-bearing; the test matrix enumerates them exhaustively. When any signal
 * is unknown (e.g. `osIdleMs === null` on platforms without an idle API), the
 * policy withholds the reload rather than guessing.
 */
export type IdleAutoReloadInputs = {
  /**
   * Milliseconds since the last OS-wide user input, or null where the platform
   * exposes no idle API. Null is treated as "cannot confirm away" — never fire.
   */
  osIdleMs: number | null;
  /** Milliseconds since this renderer session loaded. */
  sessionAgeMs: number;
  /** True while the window is visible AND focused; the reload waits for away. */
  appFocused: boolean;
  /**
   * True while any volatile, reload-destroyed work is outstanding: a huddle
   * starting/active, a foreground or background media upload (including its
   * completion settling window), queued local attachments a draft cannot
   * serialize, or an in-flight send/mutation/native operation. One blocker so
   * a new volatile-work class is guarded by extending its single reader, not by
   * threading another gate through this predicate.
   */
  volatileWorkPending: boolean;
};

/** OS idle the reload waits for: long enough that the user has clearly stepped
 * away rather than paused to read. */
export const IDLE_RELOAD_OS_IDLE_MS = 30 * 60 * 1000;

/** Minimum renderer session age before the backstop arms: below this the heap
 * and timer churn have not grown enough to be worth a reload. */
export const IDLE_RELOAD_SESSION_AGE_MS = 12 * 60 * 60 * 1000;

/**
 * Decide whether the idle backstop should reload the renderer now.
 *
 * Returns true only when the session is old, the user is provably away, and no
 * volatile work is outstanding. Any red gate returns false.
 */
export function shouldIdleReload(inputs: IdleAutoReloadInputs): boolean {
  const { osIdleMs, sessionAgeMs, appFocused, volatileWorkPending } = inputs;

  // The single volatile-work blocker: anything a reload would destroy.
  if (volatileWorkPending) return false;

  // Preconditions: only reload a stale session whose user has stepped away.
  if (appFocused) return false;
  if (sessionAgeMs < IDLE_RELOAD_SESSION_AGE_MS) return false;
  if (osIdleMs === null) return false;
  return osIdleMs >= IDLE_RELOAD_OS_IDLE_MS;
}

/**
 * Normalize a raw OS-idle read into the milliseconds the policy consumes.
 *
 * The platform reader ({@link getOsIdleSeconds}) reports seconds, null where no
 * idle API exists, and rejects when the IPC call fails. All three "cannot
 * confirm away" outcomes collapse to null so the policy withholds the reload
 * rather than guessing. Pure and reader-injected so the rejection path is
 * testable without importing the hook (React, timers, IPC).
 */
export async function normalizeOsIdleMs(
  getOsIdleSeconds: () => Promise<number | null>,
): Promise<number | null> {
  try {
    const seconds = await getOsIdleSeconds();
    return seconds === null ? null : seconds * 1000;
  } catch {
    return null;
  }
}

/**
 * Injectable signal sources for {@link runIdleReloadCheck}. The hook wires
 * these to live app state; tests wire them to fixtures — no React, timers, or
 * IPC required to exercise the orchestration.
 */
export type IdleAutoReloadReaders = {
  now: () => number;
  /** Wall-clock ms when this renderer session loaded. */
  sessionLoadedAtMs: number;
  isAppFocused: () => boolean;
  /** Async OS idle read; resolves null where unknown/unsupported. */
  getOsIdleMs: () => Promise<number | null>;
  /** Async huddle read; resolves true when a call is in progress OR unknown. */
  getHuddleActive: () => Promise<boolean>;
  /** Synchronous read of non-huddle volatile work (uploads, queued files,
   * in-flight sends/mutations/native operations). */
  isVolatileWorkPending: () => boolean;
  reload: () => Promise<void>;
};

/**
 * Gather the current signals and reload the renderer iff {@link shouldIdleReload}
 * says so. Returns whether the reload fired.
 *
 * The session-age and focus gates run before the async IPC reads so a present
 * user never triggers per-minute huddle/idle queries. The synchronous signals
 * are re-read when the decision is made so state that changed during the IPC
 * awaits (the user returning or starting work mid-check) still blocks the
 * reload.
 */
export async function runIdleReloadCheck(
  readers: IdleAutoReloadReaders,
): Promise<boolean> {
  const sessionAgeMs = readers.now() - readers.sessionLoadedAtMs;
  if (sessionAgeMs < IDLE_RELOAD_SESSION_AGE_MS) return false;
  if (readers.isAppFocused()) return false;

  const osIdleMs = await readers.getOsIdleMs();
  const huddleActive = await readers.getHuddleActive();

  const fire = shouldIdleReload({
    osIdleMs,
    sessionAgeMs: readers.now() - readers.sessionLoadedAtMs,
    appFocused: readers.isAppFocused(),
    volatileWorkPending: huddleActive || readers.isVolatileWorkPending(),
  });
  if (!fire) return false;

  await readers.reload();
  return true;
}

/**
 * Wrap {@link runIdleReloadCheck} so overlapping interval ticks collapse to one
 * in-flight check: a tick that lands while the previous check is still awaiting
 * its IPC reads (or reload) is dropped. Keeps the per-minute poll from stacking
 * concurrent huddle/idle queries. Pure — no React or timers — so the collapse
 * is exercised directly in tests.
 */
export function createSerializedIdleReloadCheck(
  readers: IdleAutoReloadReaders,
): () => void {
  let running = false;
  return () => {
    if (running) return;
    running = true;
    void runIdleReloadCheck(readers).finally(() => {
      running = false;
    });
  };
}
