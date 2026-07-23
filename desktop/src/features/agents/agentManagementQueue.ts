import type { AgentManagementRequest } from "./agentManagement";

/**
 * A draft accepted for owner review, paired with the pubkey of the managed
 * agent that sent it. The pubkey travels with the draft so the origin check on
 * submit validates the agent that actually authored the draft currently shown,
 * not whichever draft happened to arrive last.
 */
export type QueuedAgentManagementRequest = {
  agentPubkey: string;
  request: AgentManagementRequest;
};

/**
 * Decides where a newly accepted draft goes. Only one review dialog is visible
 * at a time; when a draft is already pending the incoming one is appended to a
 * FIFO queue so a burst of concurrent drafts is reviewed one after another
 * instead of the extras being silently dropped.
 *
 * Returns `"show"` when the draft should open the dialog immediately (nothing
 * pending) or `"enqueue"` when it should wait its turn. This function is pure:
 * the caller owns the queue mutation so it stays a plain ref in the hook.
 */
export function placeAcceptedRequest(
  hasPendingRequest: boolean,
): "show" | "enqueue" {
  return hasPendingRequest ? "enqueue" : "show";
}

/**
 * Pops the next draft to show once the active one is resolved. Mutates `queue`
 * by removing and returning its head (FIFO), or `undefined` when empty. Kept as
 * a tiny helper so the ordering contract (first-in shows first) is unit-tested
 * independently of React state.
 */
export function takeNextQueuedRequest(
  queue: QueuedAgentManagementRequest[],
): QueuedAgentManagementRequest | undefined {
  return queue.shift();
}
