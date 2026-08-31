// Temporary renderer profiling harness (never merges) — sampling math.
//
// Pure, deterministic decision helpers for the two DOM-driven probes so the
// harness wiring stays thin and the judgement is unit-testable without loading
// the harness's Tauri/store imports:
//   - main-thread stalls, inferred from timer drift (WebKit has no Long Tasks
//     API): a timer armed for `intervalMs` that fires late by more than
//     `thresholdMs` means the main thread was blocked for roughly that overage;
//   - input latency, split into the pre-dispatch queue delay and the
//     post-receipt paint delay.

/** Default sampling interval for the drift timer. */
export const DRIFT_INTERVAL_MS = 500;
/** Report a stall only when drift exceeds this — filters normal scheduler jitter. */
export const DRIFT_THRESHOLD_MS = 50;
/** Record an input event only when its total felt latency reaches this. */
export const INPUT_LATENCY_MIN_MS = 100;
/**
 * Largest plausible pre-dispatch age. A larger (or negative) `event.timeStamp`
 * → `performance.now()` delta means the two are on different clock bases, so
 * the queue figure is dropped rather than trusted.
 */
export const MAX_INPUT_QUEUED_MS = 60_000;

/**
 * Overage of an observed timer fire versus its expected fire time.
 *
 * `expected` is `armedAt + intervalMs`; `observed` is the actual fire time.
 * Returns the non-negative main-thread block estimate, or `null` when the
 * fire is within threshold (no stall worth recording).
 */
export function computeDrift(
  observed: number,
  expected: number,
  thresholdMs: number = DRIFT_THRESHOLD_MS,
): number | null {
  const drift = observed - expected;
  return drift > thresholdMs ? drift : null;
}

/**
 * Judge one drift interval and re-arm the baseline.
 *
 * `eligible` is false whenever the interval did not both begin and end in the
 * visible, focused foreground (a hidden/occluded/asleep webview defers timers,
 * and that deferral must never be attributed to a main-thread block). An
 * ineligible interval records nothing but still re-arms `armedAt` to `observed`
 * so the first foreground interval after a return cannot manufacture a phantom
 * stall from the accumulated background gap.
 */
export function nextStallSample(
  armedAt: number,
  observed: number,
  eligible: boolean,
  intervalMs: number = DRIFT_INTERVAL_MS,
  thresholdMs: number = DRIFT_THRESHOLD_MS,
): { dur: number | null; armedAt: number } {
  const dur = eligible
    ? computeDrift(observed, armedAt + intervalMs, thresholdMs)
    : null;
  return { dur, armedAt: observed };
}

/**
 * Split an input event's felt latency into its two additive parts and decide
 * whether it clears the recording threshold.
 *
 * `latency` is `paintedAt - receivedAt`: the capture-phase listener's receipt
 * (a `performance.now()` read taken synchronously in the listener) to the next
 * painted frame. `queued` is `receivedAt - eventTimeStamp`: the pre-dispatch
 * delay — the dominant signal when a busy main thread stalls event dispatch —
 * or `null` when that delta is implausible (a clock-basis mismatch). The event
 * is recorded when the total felt latency (`latency + queued`) reaches `minMs`.
 */
export function classifyInput(
  eventTimeStamp: number,
  receivedAt: number,
  paintedAt: number,
  minMs: number = INPUT_LATENCY_MIN_MS,
): { latency: number; queued: number | null } | null {
  const latency = paintedAt - receivedAt;
  const rawQueued = receivedAt - eventTimeStamp;
  const queued =
    rawQueued >= 0 && rawQueued <= MAX_INPUT_QUEUED_MS ? rawQueued : null;
  const felt = latency + (queued ?? 0);
  return felt >= minMs ? { latency, queued } : null;
}
