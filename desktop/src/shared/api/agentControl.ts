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

// Best-effort cooperative-steal request. The harness gates its `control_result`
// ack on a successful lock acquire, so this ack only means "frame sent" — the
// `leadership_status` stream remains the source of truth for who actually
// leads. The UI must not optimistically flip on this return value.
export async function claimManagedAgentLeadership(
  pubkey: string,
  targetInstanceId: string,
): Promise<{ status: "sent" }> {
  await sendAgentObserverControl(pubkey, {
    type: "claim_leadership",
    targetInstanceId,
  });
  return { status: "sent" };
}
