import { sendPermissionDecision } from "@/shared/api/agentControl";
import { subscribeControlResults } from "@/features/agents/observerRelayStore";
import { retransmitPermissionDecision } from "./retransmitPermissionDecision";

/** Retransmit cadence: resend the decision every 2 s until acked or expired. */
const RETRANSMIT_INTERVAL_MS = 2_000;

/**
 * Fallback card lifetime (seconds) when a permission frame carries no
 * `expiresAt` — matches the harness admission window (`PERMISSION_ASK_TIMEOUT`
 * / `OBSERVER_CONTROL_FRESHNESS_SECS`). Only archived / pre-upgrade frames lack
 * the field; live frames always carry it since harness and desktop ship
 * together.
 */
const FALLBACK_CARD_LIFETIME_SECS = 300;

/**
 * Resolve the effective expiry deadline (unix seconds) for a decision.
 *
 * Prefers the card's own `expiresAt`. When absent (an archived or pre-upgrade
 * frame signed before the field existed), fall back to the frame's timestamp
 * plus the fallback lifetime, so the card's real clock — not click time —
 * anchors the deadline; a decision on a long-archived card is already past it
 * and never retransmits. When the timestamp is also unparseable, anchor to
 * `nowSecs` so the loop still terminates within one fallback window.
 */
export function resolveDecisionDeadlineSecs(
  expiresAt: number | undefined,
  frameTimestamp: string | undefined,
  nowSecs: number,
): number {
  if (typeof expiresAt === "number") return expiresAt;
  const framedAt = frameTimestamp ? Date.parse(frameTimestamp) : NaN;
  const anchorSecs = Number.isFinite(framedAt) ? framedAt / 1000 : nowSecs;
  return anchorSecs + FALLBACK_CARD_LIFETIME_SECS;
}

/**
 * Deliver a permission decision with a retransmit-until-acked loop, wiring the
 * real relay send, `control_result` subscription, and interval scheduler into
 * the pure {@link retransmitPermissionDecision} orchestrator.
 *
 * Fire-and-forget from the caller's view: the returned promise resolves when
 * the harness acknowledges the nonce, the harness returns an authoritative
 * failure (the card re-enables for retry), or the card's deadline passes.
 * The card's UI reaction (resolve / retry) is driven separately by the
 * `control_result` reducer path, so callers need not await this.
 */
export function startPermissionDecisionDelivery({
  agentPubkey,
  channelId,
  requestNonce,
  optionId,
  deadlineSecs,
}: {
  agentPubkey: string;
  channelId: string;
  requestNonce: string;
  optionId: string;
  deadlineSecs: number;
}): Promise<"acked" | "expired" | "failed"> {
  return retransmitPermissionDecision({
    requestNonce,
    send: () =>
      sendPermissionDecision(agentPubkey, channelId, requestNonce, optionId),
    subscribe: (listener) => subscribeControlResults(agentPubkey, listener),
    scheduleRetransmit: (onTick) => {
      const id = setInterval(onTick, RETRANSMIT_INTERVAL_MS);
      // Node/test environments: don't let the interval keep the process alive.
      (id as unknown as { unref?: () => void }).unref?.();
      return () => clearInterval(id);
    },
    deadlineReached: () => Date.now() / 1000 >= deadlineSecs,
  });
}
