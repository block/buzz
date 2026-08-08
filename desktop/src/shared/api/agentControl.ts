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
 * Send a `set_config_option` control frame to a running agent. The harness
 * acknowledges via a `control_result` observer frame with `type:
 * "set_config_option"`. The caller uses this ack to persist the canonical
 * value (e.g. `effort_level`) so it takes effect on the next agent spawn.
 *
 * Pass `category` when the caller knows the option category (e.g.
 * `"thought_level"` for effort picks). The harness uses it as a trust signal
 * during the pre-discovery window (before the first session/new response),
 * ensuring picks during the first turn are stored rather than silently dropped.
 *
 * Pass `nonce` to enable per-request correlation. The harness echoes the nonce
 * in all acks (immediate and final) so the Desktop can reject stale results
 * from superseded picks without relying on value equality alone.
 */
export async function sendSetConfigOption(
  pubkey: string,
  configId: string,
  value: string,
  category?: string,
  nonce?: string,
): Promise<void> {
  await sendAgentObserverControl(pubkey, {
    type: "set_config_option",
    configId,
    value,
    ...(category !== undefined ? { category } : {}),
    ...(nonce !== undefined ? { nonce } : {}),
  });
}
