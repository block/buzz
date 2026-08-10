import type { MeshServingUsage } from "@/shared/api/tauriMesh";

/**
 * Inferring inbound work — the "someone is using my machine" signal.
 *
 * mesh-llm exposes no inbound counter. `routing_metrics` is incremented only by
 * this node's own OpenAI ingress, and `fronted_request_count` merely aliases
 * `request_count`, so there is no field that counts work done for other
 * members.
 *
 * But it is still *derivable*, because two facts we do hold are enough:
 *
 *   1. `inflight` counts inference in flight on this node, whoever asked.
 *   2. `requestsServed` (= `request_count`) counts requests **we** dispatched.
 *
 * So if we are serving, there is work in flight, and our own dispatch count has
 * not moved between samples — that work is not ours. It arrived from a peer.
 * This is inference by elimination from data we already poll, not a guess.
 *
 * Its one weakness, stated so nobody mistakes it for a counter: it is sampled.
 * A request that starts and finishes entirely between two polls is invisible,
 * so the signal can undercount. It never *over*-claims, which is the direction
 * that matters — we would rather miss a flicker than assert traffic that never
 * happened.
 */

export type MeshActivitySample = {
  /** `inflight` at the time of the sample. */
  inflight: number;
  /** `requestsServed` (outbound dispatch count) at the time of the sample. */
  requestsRouted: number;
};

export function activitySample(
  usage: MeshServingUsage | null,
): MeshActivitySample | null {
  if (!usage) {
    return null;
  }
  return { inflight: usage.inflight, requestsRouted: usage.requestsServed };
}

/**
 * Is the work currently in flight coming from someone else?
 *
 * Requires a previous sample: without one we cannot tell whether our own
 * dispatch count is moving, so we decline to claim anything.
 */
export function inferInboundWork({
  isSharing,
  current,
  previous,
}: {
  isSharing: boolean;
  current: MeshActivitySample | null;
  previous: MeshActivitySample | null;
}): boolean {
  // Only a sharing node can receive inbound work at all.
  if (!isSharing || !current || !previous) {
    return false;
  }
  if (current.inflight === 0) {
    return false;
  }
  // Our own dispatch count moved, so at least some of this is ours. Attributing
  // it to a peer would be a claim we cannot support.
  return current.requestsRouted === previous.requestsRouted;
}

/**
 * Where this machine's completed requests actually ran.
 *
 * Uses the `pressure` split, which counts *completed* requests rather than
 * attempts, so it is a fact about our own consumption rather than a retry
 * artefact. Returns null before anything completes rather than claiming
 * "0 remote".
 */
export function describeRequestOrigin(
  usage: MeshServingUsage | null,
): string | null {
  if (!usage) {
    return null;
  }
  const remote = usage.remotelyServed + usage.endpointServed;
  const total = usage.locallyServed + remote;
  if (total === 0) {
    return null;
  }
  const label = (n: number, noun: string) =>
    `${n} ${n === 1 ? noun : `${noun}s`}`;
  if (remote === 0) {
    return `${label(total, "request")} ran here`;
  }
  if (usage.locallyServed === 0) {
    return `${label(remote, "request")} ran on shared compute`;
  }
  return `${remote} of ${total} requests ran on shared compute`;
}

/**
 * The one-line participation hint under the card headline.
 *
 * Ordered by what the operator most wants to know: live inbound work first
 * (their machine is earning its keep), then live work generally, then the
 * cumulative record. Never asserts inbound work without the elimination check
 * above holding.
 */
export function describeParticipationHint({
  isSharing,
  isConsuming,
  inboundWork,
  usage,
}: {
  isSharing: boolean;
  isConsuming: boolean;
  inboundWork: boolean;
  usage: MeshServingUsage | null;
}): string {
  if (isConsuming) {
    return describeRequestOrigin(usage) ?? "Using shared compute";
  }
  if (!isSharing) {
    return "Share compute to run models";
  }
  if (inboundWork) {
    return "You're sharing · serving another member";
  }
  if ((usage?.inflight ?? 0) > 0) {
    return "You're sharing · working now";
  }
  const origin = describeRequestOrigin(usage);
  return origin ? `You're sharing · ${origin}` : "You're sharing";
}
