import type { ControlResultFrame } from "@/shared/api/types";

/**
 * Resolve the outcome of a live `switch_model` across one or more channels.
 *
 * A live switch fires a `switch_model` frame per active channel and learns each
 * channel's result asynchronously over the observer relay. The fail-fast rule:
 * any terminal failure rejects the whole pick immediately; a final `switched`
 * status must arrive from every exact turn before resolving success. The
 * immediate `sent` status is deliberately non-terminal: it confirms delivery,
 * not model validation or application. If the harness never reports a final
 * result, the fallback timeout resolves `"pending"` rather than claiming the
 * model was switched.
 *
 * The counting lives here, isolated from React and the relay so it can be unit
 * tested with synthetic frames and a fake clock. The caller injects the
 * relay subscription, the per-channel sends, and the timeout scheduler.
 */
export async function awaitLiveSwitchOutcome({
  channelCount,
  modelId,
  turnIds,
  subscribe,
  sendSwitches,
  scheduleTimeout,
}: {
  /** Number of channels the switch was fired to — the success threshold. */
  channelCount: number;
  /** Model being switched to; frames for any other model are ignored. */
  modelId: string;
  /** Exact turns targeted by this request. When present, duplicate or stale
   * acknowledgements cannot satisfy another sibling turn's slot. */
  turnIds?: readonly string[];
  /** Register a control-result listener; returns an unsubscribe function. */
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  /** Fire the per-channel `switch_model` sends. Resolves when all are sent. */
  sendSwitches: () => Promise<void>;
  /** Schedule the no-reply fallback; returns a cancel function. */
  scheduleTimeout: (onTimeout: () => void) => () => void;
}): Promise<"ok" | "pending" | "unsupported" | "failed"> {
  let unsubscribe = () => {};
  let cancelTimeout = () => {};
  let finished = false;
  let resolveSettled: (
    outcome: "ok" | "pending" | "unsupported" | "failed",
  ) => void = () => {};
  const settled = new Promise<"ok" | "pending" | "unsupported" | "failed">(
    (resolve) => {
      resolveSettled = resolve;
    },
  );
  let remaining = channelCount;
  const pendingTurnIds = turnIds ? new Set(turnIds) : null;
  const cleanup = () => {
    cancelTimeout();
    unsubscribe();
  };
  const finish = (outcome: "ok" | "pending" | "unsupported" | "failed") => {
    if (finished) return;
    finished = true;
    cleanup();
    resolveSettled(outcome);
  };

  unsubscribe = subscribe((frame) => {
    if (frame.type !== "switch_model" || frame.modelId !== modelId) {
      return;
    }
    const isSuccess = frame.status === "switched";
    const isDeliveryAck = frame.status === "sent";
    const isUnsupported = frame.status === "unsupported_model";
    const isFailed = [
      "no_active_turn",
      "ambiguous",
      "turn_ending",
      "switch_failed",
    ].includes(frame.status);
    if (!isSuccess && !isDeliveryAck && !isUnsupported && !isFailed) {
      return;
    }
    if (isDeliveryAck) {
      return;
    }
    if (pendingTurnIds) {
      if (!frame.turnId || !pendingTurnIds.delete(frame.turnId)) {
        return;
      }
      remaining = pendingTurnIds.size;
    }
    if (isUnsupported) {
      finish("unsupported");
      return;
    }
    if (isFailed) {
      finish("failed");
      return;
    }
    if (!pendingTurnIds) {
      remaining -= 1;
    }
    if (remaining <= 0) {
      finish("ok");
    }
  });
  cancelTimeout = scheduleTimeout(() => finish("pending"));

  try {
    await sendSwitches();
  } catch {
    // Delivery is not atomic: one relay publish can succeed before a sibling
    // publish fails. Treat that as the same partial-application failure as a
    // rejected harness result so the caller refreshes every session surface
    // and never implies the previous model was retained everywhere.
    finish("failed");
  }

  return settled;
}

/**
 * Attempt every live model-switch publish even when an earlier target fails.
 * Relay sends are independent and cannot be rolled back, so fail only after
 * every exact turn has had a delivery attempt.
 */
export async function sendAllLiveSwitchRequests(
  requests: readonly (() => Promise<void>)[],
): Promise<void> {
  const results = await Promise.allSettled(
    requests.map((request) => Promise.resolve().then(request)),
  );
  const failures = results.filter((result) => result.status === "rejected");
  if (failures.length > 0) {
    throw new AggregateError(
      failures.map((failure) => failure.reason),
      `Failed to deliver ${failures.length} of ${requests.length} live model-switch requests`,
    );
  }
}
