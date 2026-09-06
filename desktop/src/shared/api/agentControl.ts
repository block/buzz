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

/**
 * Send a permission decision to a running agent's ACP harness. The decision
 * is fire-and-forget: the harness receives it via the observer control channel
 * and updates the permission card asynchronously via a `control_result` frame.
 *
 * @param pubkey    - Agent's public key (hex or npub).
 * @param channelId - The channel from which the permission request was issued.
 *                    The harness validates this before looking up the nonce.
 * @param nonce     - `requestNonce` from the `authorization` envelope on the
 *                    corresponding `acp_read` permission frame.
 * @param optionId  - The chosen option's `optionId` (e.g. `"allow_once"`).
 */
export async function sendPermissionDecision(
  pubkey: string,
  channelId: string,
  nonce: string,
  optionId: string,
): Promise<void> {
  await sendAgentObserverControl(pubkey, {
    type: "permission_decision",
    channelId,
    requestNonce: nonce,
    optionId,
  });
}
