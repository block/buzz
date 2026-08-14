import type { ControlResultFrame } from "@/shared/api/types";

/**
 * Resolve the outcome of a live `switch_model` across one or more channels.
 *
 * A live switch fires a `switch_model` frame per active channel and learns each
 * channel's result asynchronously over the observer relay. The fail-fast rule:
 * any single `unsupported_model` result rejects the whole pick immediately;
 * every other status must arrive from every channel before resolving success.
 * If the harness never replies, the fallback timeout resolves `"ok"` — the
 * override still rides the requeued/next session, we just can't confirm it
 * synchronously.
 *
 * The counting lives here, isolated from React and the relay so it can be unit
 * tested with synthetic frames and a fake clock. The caller injects the
 * relay subscription, the per-channel sends, and the timeout scheduler.
 */
export async function awaitLiveSwitchOutcome({
  channelCount,
  modelId,
  subscribe,
  sendSwitches,
  scheduleTimeout,
}: {
  /** Number of channels the switch was fired to — the success threshold. */
  channelCount: number;
  /** Model being switched to; frames for any other model are ignored. */
  modelId: string;
  /** Register a control-result listener; returns an unsubscribe function. */
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  /** Fire the per-channel `switch_model` sends. Resolves when all are sent. */
  sendSwitches: () => Promise<void>;
  /** Schedule the no-reply fallback; returns a cancel function. */
  scheduleTimeout: (onTimeout: () => void) => () => void;
}): Promise<"ok" | "unsupported"> {
  const settled = new Promise<"ok" | "unsupported">((resolve) => {
    let unsubscribe = () => {};
    let cancelTimeout = () => {};
    let remaining = channelCount;
    const finish = (outcome: "ok" | "unsupported") => {
      cancelTimeout();
      unsubscribe();
      resolve(outcome);
    };
    cancelTimeout = scheduleTimeout(() => finish("ok"));
    unsubscribe = subscribe((frame) => {
      if (frame.type !== "switch_model" || frame.modelId !== modelId) {
        return;
      }
      if (frame.status === "unsupported_model") {
        // Any single failure rejects the whole pick immediately.
        finish("unsupported");
        return;
      }
      // sent / switched / turn_ending — count as success for this channel.
      remaining -= 1;
      if (remaining <= 0) {
        finish("ok");
      }
    });
  });

  await sendSwitches();

  return settled;
}
