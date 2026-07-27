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
 * Resolve one live ACP permission request. The harness accepts only
 * owner-signed, encrypted controls that match the exact channel, turn, and
 * JSON-RPC request id, then verifies that optionId belongs to that request.
 */
export async function resolveManagedAgentPermission(
  pubkey: string,
  channelId: string,
  turnId: string,
  requestId: string | number,
  optionId: string,
): Promise<void> {
  await sendAgentObserverControl(pubkey, {
    type: "permission_decision",
    channelId,
    turnId,
    requestId,
    optionId,
  });
}
