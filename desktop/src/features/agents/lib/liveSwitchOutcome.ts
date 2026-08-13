import type { ControlResultFrame } from "@/shared/api/types";

/**
 * Resolve the outcome of a live `switch_model` across one or more channels.
 *
 * A live switch fires a `switch_model` frame per active channel and learns each
 * channel's result asynchronously over the observer relay. Two statuses
 * fail-fast — any single frame rejects the whole pick immediately, without
 * waiting for the other channels or the timeout:
 *   - `unsupported_model` → the target model isn't available for this agent.
 *   - `failure`           → the adapter refused the switch (the session stays
 *                           on its current model).
 * Their causes differ, so they resolve to distinct outcomes (`"unsupported"`
 * vs `"failed"`) the caller can message separately.
 *
 * `sent` is the busy-path PROVISIONAL ack: the switch was delivered to the
 * in-flight turn, but the adapter isn't consulted until the requeued session
 * runs. The real verdict lands later as a positive `switched` terminal (the
 * deferred apply succeeded), a `failure`/`unsupported_model` frame (it didn't),
 * so `sent` never settles the pick on its own — the subscription stays alive
 * for the terminal frame.
 *
 * Success is only ever inferred from a real positive terminal frame
 * (`switched` / `turn_ending` / `no_active_turn`), which must arrive from every
 * EXPECTED channel before resolving `"ok"`. The idle path emits its terminal
 * immediately; the busy path emits `switched` when the requeued session applies
 * the model. A busy turn routinely outlasts the fallback timeout, so the
 * timeout NEVER resolves `"ok"` — it resolves `"pending"`: the switch was
 * accepted and rides the requeued/next session, but we could not confirm the
 * apply synchronously. The caller surfaces that truthfully rather than claiming
 * a success that has not happened (and might yet be rejected).
 *
 * Two identity guards keep a stale or replayed frame from settling the wrong
 * pick. The observer relay requests a five-minute replay on reconnect, so an
 * old `control_result` for an earlier switch can re-arrive mid-pick:
 *   - `requestId` — an opaque per-pick correlator the harness echoes on every
 *     frame. Frames without a matching id are ignored, so a replayed result for
 *     a prior operation (which carried a different id, or none) is inert.
 *   - `channelId` — every frame, positive OR negative, must name a channel in
 *     the EXPECTED set before it can settle anything. A misrouted `failure`
 *     from a foreign channel (or a frame with no channel) can no more fail the
 *     pick than a foreign `switched` can satisfy it. Positive terminals are
 *     then counted once per DISTINCT expected channel, not once per frame, so
 *     two copies of one channel's `switched` can't satisfy a two-channel pick.
 *
 * The counting lives here, isolated from React and the relay so it can be unit
 * tested with synthetic frames and a fake clock. The caller injects the
 * relay subscription, the per-channel sends, and the timeout scheduler.
 */
export async function awaitLiveSwitchOutcome({
  requestId,
  channelIds,
  subscribe,
  sendSwitches,
  scheduleTimeout,
}: {
  /** Opaque per-pick id; frames without this exact id are ignored. */
  requestId: string;
  /** Channels the switch was fired to — the distinct set to await. */
  channelIds: readonly string[];
  /** Register a control-result listener; returns an unsubscribe function. */
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  /** Fire the per-channel `switch_model` sends. Resolves when all are sent. */
  sendSwitches: () => Promise<void>;
  /** Schedule the no-reply fallback; returns a cancel function. */
  scheduleTimeout: (onTimeout: () => void) => () => void;
}): Promise<"ok" | "unsupported" | "failed" | "pending"> {
  const expected = new Set(channelIds);
  const settled = new Promise<"ok" | "unsupported" | "failed" | "pending">(
    (resolve) => {
      let unsubscribe = () => {};
      let cancelTimeout = () => {};
      const succeeded = new Set<string>();
      const finish = (outcome: "ok" | "unsupported" | "failed" | "pending") => {
        cancelTimeout();
        unsubscribe();
        resolve(outcome);
      };
      // No positive terminal in time: the switch was accepted but its deferred
      // apply hasn't confirmed. Resolve indeterminate — never a false "ok".
      cancelTimeout = scheduleTimeout(() => finish("pending"));
      unsubscribe = subscribe((frame) => {
        // Two identity guards run BEFORE any status handling, so they scope
        // every decision — positive AND negative — to THIS pick's channels:
        //   - requestId: a replayed result for a prior operation carries a
        //     different id (or none) and is ignored.
        //   - channelId: the frame must name an EXPECTED channel. A negative
        //     frame (`failure`/`unsupported_model`) misrouted from a foreign
        //     channel, or carrying no channel at all, must not fail this pick
        //     any more than a foreign positive frame may satisfy it.
        if (frame.type !== "switch_model" || frame.requestId !== requestId) {
          return;
        }
        if (!frame.channelId || !expected.has(frame.channelId)) {
          return;
        }
        if (frame.status === "unsupported_model") {
          // Model unavailable — reject the whole pick immediately.
          finish("unsupported");
          return;
        }
        if (frame.status === "failure") {
          // Adapter refused the switch — reject immediately. The session stays
          // on its current model; distinct outcome so the caller can say why.
          finish("failed");
          return;
        }
        if (frame.status === "sent") {
          // Busy-path provisional ack: the switch was delivered to the in-flight
          // turn, but the adapter isn't consulted until the requeued session
          // runs. Don't count it — the real positive `switched` terminal (or a
          // `failure`/`unsupported_model`) settles the pick; if none arrives in
          // time the timeout resolves `"pending"`.
          return;
        }
        // switched / turn_ending / no_active_turn — positive terminal for this
        // channel. Count each expected channel once: a duplicate frame for a
        // channel already recorded (a replay, or a two-copy fan-out) is a no-op.
        succeeded.add(frame.channelId);
        if (succeeded.size >= expected.size) {
          finish("ok");
        }
      });
    },
  );

  await sendSwitches();

  return settled;
}
