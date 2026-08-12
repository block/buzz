import { sendAgentObserverControl } from "@/shared/api/observerRelay";
import type { CancelManagedAgentTurnResult } from "@/shared/api/types";

export async function cancelManagedAgentTurn(
  pubkey: string,
  channelId: string,
): Promise<CancelManagedAgentTurnResult> {
  await sendAgentObserverControl(pubkey, {
    type: "cancel_turn",
    channelId,
  });
  return { status: "sent" };
}

/**
 * Send a live model-switch control frame to a running agent. The switch rides
 * the harness's cancel-switch-requeue path (busy turn) or invalidate-and-reapply
 * (idle); the outcome arrives asynchronously as a `control_result` observer
 * frame, not as the return value here. This is fire-and-forget on the send side.
 */
export async function switchManagedAgentModel(
  pubkey: string,
  channelId: string,
  modelId: string,
): Promise<void> {
  await sendAgentObserverControl(pubkey, {
    type: "switch_model",
    channelId,
    modelId,
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
