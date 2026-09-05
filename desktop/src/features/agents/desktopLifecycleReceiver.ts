import type { LiveSubscriptionClosedRecovery } from "@/shared/api/relayClientShared";
import { waitForRateLimit } from "@/shared/api/relayRateLimitGate";
import type { DesktopScope } from "./desktopList";
import { receiveLifecycle } from "./desktopLifecycle";
import {
  LifecycleReceiverError,
  receiverErrorMessage,
} from "./desktopLifecycleDiagnostics";

export const RECEIVER_RECOVERY_DELAYS_MS = [1_000, 2_000, 4_000] as const;

const CLOSED_MESSAGE =
  "Desktop lifecycle receiver subscription closed. Retry the receiver to accept new requests.";

type StartReceiver = (
  scope: DesktopScope,
  active: () => boolean,
  onError: (message: string) => void,
  onReady: () => void,
  onClosed: (recovery: LiveSubscriptionClosedRecovery) => void,
) => Promise<() => void>;

type ReceiverOwnerDependencies = {
  startReceiver?: StartReceiver;
  waitForRateLimit?: () => Promise<void>;
  setTimer?: (callback: () => void, delayMs: number) => number;
  clearTimer?: (timer: number) => void;
};

/**
 * Owns one lifecycle receiver scope. Recovery always creates a fresh live-only
 * subscription and lets receiveLifecycle repeat projection sync before it
 * admits events. The attempt budget belongs to this owner and is intentionally
 * not reset by a successful EOSE/sync followed by another CLOSED.
 */
export function ownLifecycleReceiver(
  scope: DesktopScope,
  onError: (message: string) => void,
  onReady: () => void,
  dependencies: ReceiverOwnerDependencies = {},
) {
  const startReceiver: StartReceiver =
    dependencies.startReceiver ??
    ((receiverScope, active, reportError, ready, closed) =>
      receiveLifecycle(
        receiverScope,
        active,
        reportError,
        undefined,
        undefined,
        ready,
        closed,
      ));
  const waitForGate = dependencies.waitForRateLimit ?? waitForRateLimit;
  const setTimer =
    dependencies.setTimer ??
    ((callback, delayMs) => window.setTimeout(callback, delayMs));
  const clearTimer =
    dependencies.clearTimer ?? ((timer) => window.clearTimeout(timer));

  let stopped = false;
  let generation = 0;
  let recoveryAttempt = 0;
  let timer: number | undefined;
  let closeCurrent: (() => void) | undefined;

  const current = (token: number) => !stopped && generation === token;

  const retireCurrent = () => {
    const close = closeCurrent;
    closeCurrent = undefined;
    close?.();
  };

  const recover = (
    token: number,
    terminalMessage: string,
    recovery: LiveSubscriptionClosedRecovery,
  ) => {
    if (!current(token)) return;
    generation++;
    retireCurrent();

    if (
      recovery.classification === "terminal" ||
      recoveryAttempt >= RECEIVER_RECOVERY_DELAYS_MS.length
    ) {
      onError(terminalMessage);
      return;
    }

    const delayMs = Math.max(
      RECEIVER_RECOVERY_DELAYS_MS[recoveryAttempt],
      recovery.retryAfterMs,
    );
    recoveryAttempt++;
    const waitingGeneration = generation;
    timer = setTimer(() => {
      timer = undefined;
      if (stopped || generation !== waitingGeneration) return;
      void waitForGate().then(() => {
        if (!stopped && generation === waitingGeneration) start();
      });
    }, delayMs);
  };

  const start = () => {
    const token = ++generation;
    void startReceiver(
      scope,
      () => current(token),
      (message) => {
        if (current(token)) onError(message);
      },
      () => {
        if (current(token)) onReady();
      },
      (recovery) => recover(token, CLOSED_MESSAGE, recovery),
    )
      .then((close) => {
        if (current(token)) closeCurrent = close;
        else close();
      })
      .catch((error) => {
        if (current(token))
          recover(token, receiverErrorMessage(error), {
            classification:
              error instanceof LifecycleReceiverError
                ? error.recoveryClassification
                : "retryable",
            retryAfterMs: 0,
          });
      });
  };

  start();

  return () => {
    if (stopped) return;
    stopped = true;
    generation++;
    if (timer !== undefined) {
      clearTimer(timer);
      timer = undefined;
    }
    retireCurrent();
  };
}
