/** Safe receiver diagnostics: raw IPC/transport errors can contain private data. */
type Stage =
  | "initialization"
  | "subscription"
  | "history"
  | "projection"
  | "reconciliation";

export class LifecycleReceiverError extends Error {
  constructor(stage: Stage, error: unknown) {
    const value = error instanceof Error ? error.message : error;
    const reason =
      value === "closed"
        ? "subscription closed"
        : value === "Desktop lifecycle scope changed"
          ? "scope changed"
          : value === "Relay session is terminal; cannot reconnect."
            ? "relay session requires reconnection"
            : value === "Timed out while loading channel history."
              ? "history timed out"
              : "request failed";
    super(`Desktop lifecycle receiver is unavailable (${stage}: ${reason}).`);
  }
}

export function receiverErrorMessage(error: unknown): string {
  return error instanceof LifecycleReceiverError
    ? error.message
    : "Desktop lifecycle receiver is unavailable (initialization failed).";
}

export async function receiverStep<T>(
  stage: Stage,
  action: () => Promise<T>,
): Promise<T> {
  try {
    return await action();
  } catch (error) {
    throw new LifecycleReceiverError(stage, error);
  }
}
