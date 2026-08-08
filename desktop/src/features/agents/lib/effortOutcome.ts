import type { ControlResultFrame } from "@/shared/api/types";

/**
 * Await the outcome of an effort set (or clear) request.
 *
 * Sends a `set_config_option` frame and waits for the matching final
 * `control_result` from the harness. Two phases:
 *
 *   1. Immediate ack from `handle_set_config_option_control`:
 *      `pending_session` (stored, will apply at next session — for both picks
 *      and clears) or `invalid_value` (rejected by harness validation).
 *
 *   2. Final ack from the harness (`pool.resolve_effort_report`, emitted via the
 *      main loop's `PoolEvent::EffortReport` arm immediately after
 *      `session_set_config_option` resolves — before the prompt is sent):
 *      `ok` (adapter accepted; Desktop persists) or
 *      `failure` (adapter rejected or timeout) or
 *      `cleared` (session ran without effort override; Desktop persists null).
 *
 * The function resolves with the first *terminal* status received:
 *   - `"ok"` / `"failure"` / `"invalid_value"` / `"cleared"` — terminal.
 *   - `"pending_session"` — non-terminal; awaiting final result from the harness.
 *
 * Correlation: when `nonce` is provided, only acks carrying the same nonce are
 * considered. This prevents a stale ack from a superseded pick from settling
 * the current promise. Without nonce, correlation falls back to configId+value.
 *
 * If no terminal result arrives within the timeout, resolves with
 * `"pending_session"` (the effort will be applied at the next session — the UI
 * should show this as a deferred confirmation).
 */
export async function awaitEffortOutcome({
  configId,
  value,
  nonce,
  subscribe,
  send,
  scheduleTimeout,
}: {
  /** The thought_level configId from the session cache. */
  configId: string;
  /** The value being set (or "" for clear). */
  value: string;
  /** Nonce echoed by the harness in all acks for this request. When provided,
   *  used as the primary correlation key so stale acks from prior picks are
   *  ignored even if they share the same configId and value. */
  nonce?: string;
  /** Register a control-result listener; returns an unsubscribe function. */
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  /** Fire the set_config_option send. */
  send: () => Promise<void>;
  /** Schedule the no-reply fallback; returns a cancel function. */
  scheduleTimeout: (onTimeout: () => void) => () => void;
}): Promise<
  "ok" | "failure" | "invalid_value" | "cleared" | "pending_session"
> {
  type Outcome =
    | "ok"
    | "failure"
    | "invalid_value"
    | "cleared"
    | "pending_session";

  const settled = new Promise<Outcome>((resolve) => {
    let unsubscribe = () => {};
    let cancelTimeout = () => {};
    const finish = (outcome: Outcome) => {
      cancelTimeout();
      unsubscribe();
      resolve(outcome);
    };
    cancelTimeout = scheduleTimeout(() => finish("pending_session"));
    unsubscribe = subscribe((frame) => {
      if (frame.type !== "set_config_option" || frame.configId !== configId) {
        return;
      }
      // Primary correlation: nonce when provided. Rejects stale acks from
      // superseded picks even when they share configId and value.
      if (nonce !== undefined) {
        if ((frame as Record<string, unknown>).nonce !== nonce) {
          return;
        }
      } else {
        // Fallback: correlate by value for non-clear picks.
        if (value !== "" && frame.value !== value) {
          return;
        }
      }
      const s = frame.status;
      if (
        s === "ok" ||
        s === "failure" ||
        s === "invalid_value" ||
        s === "cleared"
      ) {
        finish(s);
        return;
      }
      // pending_session is non-terminal — keep waiting for the final ack.
      // The timeout will eventually fire if no final ack arrives.
    });
  });

  await send();

  return settled;
}
