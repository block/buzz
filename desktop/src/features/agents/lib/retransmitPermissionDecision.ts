import type { ControlResultFrame } from "@/shared/api/types";

/**
 * Drive a permission decision to the agent harness with a retransmit-until-acked
 * loop, so an owner's click survives a harness socket that is briefly down.
 *
 * The send side is otherwise fire-and-forget: `sendPermissionDecision` publishes
 * one `permission_decision` observer control frame and the harness replies out
 * of band with a `control_result`. Kind-24200 control frames are ephemeral — the
 * relay never stores them — so a frame that reaches the relay while the agent's
 * subscription is down is dropped and never delivered. A single fire-and-forget
 * send therefore has no delivery guarantee: the card would hang until the 300 s
 * fail-closed timeout even though the owner decided promptly.
 *
 * This orchestrator resends the decision on a fixed cadence until it observes a
 * `control_result` for THIS nonce, then stops. The outcome depends on the frame's
 * status:
 *
 * - `sent` / `already_decided` → the harness routed or previously
 *   delivery-suppressed the decision. Resolves `"acked"` (loop stops, card stays
 *   disabled while the terminal edit arrives).
 *
 * - `channel_full` → a transient queue-saturation condition (the owning read
 *   loop's 8-slot queue was full at the moment of delivery). The loop stays
 *   active and resends on the next scheduler tick; the owning loop's first-wins
 *   dedup tolerates duplicates once the queue drains. The card stays disabled
 *   during the automatic retry. Worst case: the deadline bound in
 *   `deadlineReached()` ends the loop with `"expired"` and the card fails closed
 *   exactly as it would without this retry.
 *
 * - `no_active_turn` / `channel_closed` / `no_channel` → authoritative routing
 *   refusals (no in-flight turn, channel gone). The loop resolves `"failed"`,
 *   stopping retransmission (re-sending the same nonce cannot change the refusal),
 *   and the card returns to the actionable state for owner retry.
 *
 * If no reply arrives before the card's own `expiresAt` deadline the loop
 * resolves `"expired"`: the card times out on its own and a decision applied past
 * expiry would be rejected anyway, so retransmitting past it is pointless.
 *
 * A nonce guard scopes the settling frame to this exact decision: a replayed or
 * concurrent `control_result` for a different card carries a different nonce and
 * is inert, mirroring the `requestId` guard in `awaitLiveSwitchOutcome`.
 *
 * A send rejection is the transport failure this loop exists to survive, so the
 * orchestrator never throws: the first send and every retransmit run through
 * the same guarded path, and a rejected attempt is swallowed — the next tick
 * retries. This guarantees exactly one loop per click (the UI's disabled state
 * guards double-click) and no unhandled rejection on any path. Permanent send
 * failure therefore ends the same way as silence: the loop retries until
 * `expiresAt`, then resolves `"expired"` and the card fails closed.
 *
 * The loop is isolated from React and the relay so it can be unit tested with a
 * fake clock and synthetic frames. The caller injects the send, the
 * `control_result` subscription, and the retransmit scheduler.
 */
export function retransmitPermissionDecision({
  requestNonce,
  send,
  subscribe,
  scheduleRetransmit,
  deadlineReached,
}: {
  /** Nonce of the decision being delivered; frames without it are ignored. */
  requestNonce: string;
  /** Publish one `permission_decision` frame. Resolves when the send settles. */
  send: () => Promise<void>;
  /** Register a `control_result` listener; returns an unsubscribe function. */
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  /**
   * Schedule the retransmit cadence; `onTick` fires once per interval. Returns
   * a cancel function. The caller drives the real interval (e.g. every 2 s).
   */
  scheduleRetransmit: (onTick: () => void) => () => void;
  /**
   * Whether the card's `expiresAt` deadline has passed. Checked before each
   * retransmit so the loop never resends past expiry.
   */
  deadlineReached: () => boolean;
}): Promise<"acked" | "expired" | "failed"> {
  return new Promise<"acked" | "expired" | "failed">((resolve) => {
    let unsubscribe = () => {};
    let cancelRetransmit = () => {};
    const finish = (outcome: "acked" | "expired" | "failed") => {
      cancelRetransmit();
      unsubscribe();
      resolve(outcome);
    };
    // Attempt one transmit, respecting the deadline and swallowing a rejected
    // send. A rejection is a failed transmit, not a fatal error: the next tick
    // retries, so the loop survives a socket that is briefly down.
    const transmit = () => {
      if (deadlineReached()) {
        finish("expired");
        return;
      }
      void send().catch(() => {});
    };
    unsubscribe = subscribe((frame) => {
      // A `control_result` for THIS nonce means the harness received the
      // decision. Frames for other cards (or non-permission frames) carry a
      // different nonce (or none) and are inert.
      if (
        frame.type !== "permission_decision" ||
        frame.requestNonce !== requestNonce
      ) {
        return;
      }
      // `sent` / `already_decided` → harness routed or delivery-suppressed the
      // decision; settle `acked`.
      if (frame.status === "sent" || frame.status === "already_decided") {
        finish("acked");
        return;
      }
      // `channel_full` is a transient queue-saturation signal: the owning read
      // loop's queue was momentarily full. Do NOT settle — stay subscribed and
      // let the scheduler keep resending. The card remains disabled until the
      // loop either acks or the deadline expires. The owning loop's first-wins
      // dedup tolerates the duplicate delivery once the queue drains.
      if (frame.status === "channel_full") {
        return;
      }
      // `no_active_turn` / `channel_closed` / `no_channel` are authoritative
      // routing refusals — retransmitting the same nonce cannot change them.
      // Settle `failed` so the card re-enables for owner retry.
      finish("failed");
    });
    cancelRetransmit = scheduleRetransmit(transmit);

    // Fire the first send immediately, then let the scheduler drive resends.
    transmit();
  });
}
