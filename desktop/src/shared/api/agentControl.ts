import { sendAgentObserverControl } from "@/shared/api/observerRelay";

/** Send a stop request; the harness acknowledges it via control_result. */
export async function cancelManagedAgentTurn(
  pubkey: string,
  channelId: string,
  requestId: string,
): Promise<void> {
  await sendAgentObserverControl(pubkey, {
    type: "cancel_turn",
    channelId,
    requestId,
  });
}

/**
 * Send a live model-switch control frame to a running agent. The switch rides
 * the harness's cancel-switch-requeue path (busy turn) or invalidate-and-reapply
 * (idle); the outcome arrives asynchronously as a `control_result` observer
 * frame, not as the return value here. This is fire-and-forget on the send side.
 *
 * `requestId` is an opaque per-pick correlator the harness echoes back on both
 * the immediate ack and the late terminal frame, so a reconnect replay of an
 * earlier pick's result cannot settle this one.
 */
export async function switchManagedAgentModel(
  pubkey: string,
  channelId: string,
  modelId: string,
  requestId: string,
): Promise<void> {
  await sendAgentObserverControl(pubkey, {
    type: "switch_model",
    channelId,
    modelId,
    requestId,
  });
}
