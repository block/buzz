import type { ControlResultFrame } from "@/shared/api/types";

/** Stop feedback must describe the harness result, not relay delivery alone. */
export async function awaitCancelTurnOutcome({
  requestId,
  channelId,
  subscribe,
  sendCancel,
  scheduleTimeout,
}: {
  requestId: string;
  channelId: string;
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  sendCancel: () => Promise<void>;
  scheduleTimeout: (onTimeout: () => void) => () => void;
}): Promise<"sent" | "no_active_turn" | "ambiguous_target" | "unconfirmed"> {
  type Outcome = "sent" | "no_active_turn" | "ambiguous_target" | "unconfirmed";

  let settled = false;
  let unsubscribe = () => {};
  let cancelTimeout = () => {};
  let resolveResult: (outcome: Outcome) => void = () => {};
  let rejectResult: (error: unknown) => void = () => {};
  const result = new Promise<Outcome>((resolve, reject) => {
    resolveResult = resolve;
    rejectResult = reject;
  });
  const cleanup = () => {
    unsubscribe();
    cancelTimeout();
  };
  const settle = (outcome: Outcome) => {
    if (settled) return;
    settled = true;
    cleanup();
    resolveResult(outcome);
  };
  const fail = (error: unknown) => {
    if (settled) return;
    settled = true;
    cleanup();
    rejectResult(error);
  };

  unsubscribe = subscribe((frame) => {
    if (
      frame.type !== "cancel_turn" ||
      frame.requestId !== requestId ||
      frame.channelId !== channelId
    ) {
      return;
    }
    if (
      frame.status === "sent" ||
      frame.status === "no_active_turn" ||
      frame.status === "ambiguous_target"
    ) {
      settle(frame.status);
    }
  });
  // Start the timeout before sending. A hung relay transport must not keep the
  // caller pending forever; timeout truthfully reports that the harness result
  // was not confirmed. The send promise is still observed below so a later
  // rejection cannot become an unhandled rejection.
  cancelTimeout = scheduleTimeout(() => settle("unconfirmed"));
  try {
    // Race transport failure against the harness result. A correlated result
    // may arrive before publish resolves, and a transport that hangs must not
    // block the timeout from settling the outer operation.
    void Promise.resolve(sendCancel()).catch(fail);
  } catch (error) {
    fail(error);
  }

  return result;
}
