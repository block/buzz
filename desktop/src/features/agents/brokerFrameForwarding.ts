import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

/**
 * Wire `type` discriminator on a broker request payload. Must match
 * `BROKER_REQUEST_TYPE` in `crates/buzz-sdk/src/broker/mod.rs`.
 */
export const BROKER_REQUEST_TYPE = "broker_request";

export type BrokerFrameHandled = {
  brokerRequest: boolean;
  requestId: string | null;
  resultDelivered: boolean;
};

/**
 * True when a decrypted observer payload is a broker request rather than agent
 * telemetry.
 *
 * This is a **routing** decision, not a trust decision. Rust re-verifies the
 * signature, re-derives the owner and requester from the frame, re-decrypts, and
 * re-validates before anything executes — so a wrong answer here can only cause
 * a broker request to be missed (the requester's wait expires) or a non-request
 * to be forwarded and ignored. It cannot cause an unauthorized mutation.
 *
 * The check is deliberately shallow: the renderer must not decide whether a
 * request is *valid*, only whether it claims to be one. An ill-formed broker
 * request has to reach the host so the requester gets a structured refusal
 * instead of silence.
 */
export function isBrokerRequestPayload(payload: unknown): boolean {
  return (
    typeof payload === "object" &&
    payload !== null &&
    (payload as { type?: unknown }).type === BROKER_REQUEST_TYPE
  );
}

/**
 * Hand a signed observer frame to the Rust broker for execution.
 *
 * The **raw signed event** is forwarded, not the decrypted payload: the host
 * must verify and decrypt it itself, from bytes the requester signed. Passing
 * the plaintext would make the renderer the thing that decides what a request
 * says, which is precisely the authority split the broker exists to avoid.
 */
export async function forwardBrokerFrame(
  event: RelayEvent,
): Promise<BrokerFrameHandled> {
  return invokeTauri<BrokerFrameHandled>("broker_handle_observer_frame", {
    eventJson: JSON.stringify(event),
  });
}

/**
 * Forward a broker frame without letting a host fault escape to the caller.
 *
 * The observer subscription carries agent telemetry for the whole app. A broker
 * host fault is a different subsystem's failure, so it must not surface as an
 * observer transport error — doing so would report a misleading cause and tear
 * down telemetry for every agent over a failed mutation. The requester is
 * unaffected either way: it learns the outcome from its own signed result frame,
 * or hears nothing and retries with the same `requestId`.
 *
 * Returns `null` when forwarding failed.
 */
export async function forwardBrokerFrameIsolated(
  event: RelayEvent,
): Promise<BrokerFrameHandled | null> {
  try {
    return await forwardBrokerFrame(event);
  } catch (error) {
    console.error("Broker frame forwarding failed", error);
    return null;
  }
}
