import type {
  MeshLiveView,
  MeshServingUsage,
  MeshSnapshot,
} from "@/shared/api/tauriMesh";
import { describeRequestOrigin } from "./meshActivity";

/**
 * Pure projection for the mesh detail popover.
 *
 * The popover answers three questions the 256px card cannot:
 *   1. Who is on the mesh right now, and how much do they bring?
 *   2. Is the spice flowing — is work actually happening?
 *   3. Am I running on someone else's machine, or my own?
 *
 * Peers come from the **live gossip view**, not the relay snapshot. Status notes
 * stay valid for 120s and so outlive the node that wrote them; a topology built
 * on notes shows devices gossip already knows are gone. The relay snapshot is
 * used only when no local runtime exists, where it is the sole available view.
 *
 * The hard constraint this file exists to encode: **mesh-llm exposes no inbound
 * counter.** `routing_metrics` is incremented only by this node's own OpenAI
 * ingress, so every number available describes work *this machine dispatched*.
 * Nothing can prove another member consumed our compute. So:
 *
 *   - "I used someone else's machine"  → provable (`remotelyServed`)
 *   - "my own GPU did the work"        → provable (`locallyServed`)
 *   - "someone used MY machine"        → no counter; INFERRED by elimination
 *
 * That last one is derivable without a counter: sharing, with work in flight,
 * while our own dispatch count stays flat, means the work is not ours. See
 * `inferInboundWork` in `meshActivity.ts`. It is sampled, so it can undercount
 * — but it never over-claims, which is the direction that matters.
 *
 * ## The cold state explains, it does not report
 *
 * With nothing shared anywhere there is no status worth stating, so the subtitle
 * says what the switch does rather than restating a zero. Every other state has
 * a real number, and an explanation sitting permanently beside those would be
 * furniture.
 */

export type MeshDetailModel = {
  /** e.g. "115 GB" — live pool capacity, or null when nothing is shared. */
  capacityLabel: string | null;
  /** e.g. "2 peers connected", or the relay view when not participating. */
  participationLabel: string;
  /** True when inference is in flight on this node right now. */
  busyNow: boolean;
  /**
   * Live activity phrase, or null when idle. Only claims inbound work when the
   * elimination check in `meshActivity.ts` holds.
   */
  activityLabel: string | null;
  /** Where this machine's completed requests actually ran. */
  originLabel: string | null;
  /** Distinct models serving across the live mesh. */
  modelCount: number;
  /** True when a local runtime is up, so peers are knowable at all. */
  connected: boolean;
};

/**
 * The live "spice is flowing" phrase.
 *
 * Wording is intentionally agent-agnostic: `inflight` counts work in flight on
 * this node without distinguishing a local agent from a remote member, so this
 * may never assert that someone else is using this machine.
 */
export function describeActivity({
  usage,
  isSharing,
  inboundWork,
}: {
  usage: MeshServingUsage | null;
  isSharing: boolean;
  inboundWork: boolean;
}): string | null {
  const inflight = usage?.inflight ?? 0;
  if (inflight > 0) {
    const requests = `${inflight} ${inflight === 1 ? "request" : "requests"} in flight`;
    // Only name a peer as the source when elimination proves it isn't ours.
    return inboundWork ? `${requests} · from another member` : requests;
  }
  if (!usage) {
    return null;
  }
  // Sharing with a warm runtime and no traffic is a real, distinct state:
  // ready, but nothing has asked yet.
  if (isSharing && usage.requestsServed === 0) {
    return "Ready · no requests yet";
  }
  return null;
}

export function deriveMeshDetailModel({
  view,
  snapshot,
  usage,
  isSharing,
  inboundWork,
}: {
  view: MeshLiveView | null;
  snapshot: MeshSnapshot | null;
  usage: MeshServingUsage | null;
  isSharing: boolean;
  inboundWork: boolean;
}): MeshDetailModel {
  const connected = view?.connected === true;
  const peers = view?.peers ?? [];

  // Capacity from the live mesh: this machine plus serving peers. A consuming
  // peer contributes presence but no GB, because it shares none.
  const liveCapacity = connected
    ? [
        view?.selfCapacityGb ?? 0,
        ...peers.map((peer) => peer.capacityGb ?? 0),
      ].reduce((total, gb) => total + gb, 0)
    : 0;
  const capacityGb = connected
    ? liveCapacity > 0
      ? liveCapacity
      : null
    : (snapshot?.sharedCapacityGb ?? null);

  const models = new Set(peers.flatMap((peer) => peer.models));
  if (isSharing) {
    for (const model of snapshot?.devices.find((d) => d.isSelf)?.models ?? []) {
      models.add(model);
    }
  }

  return {
    connected,
    capacityLabel:
      capacityGb === null
        ? null
        : `${capacityGb >= 10 ? Math.round(capacityGb) : Math.round(capacityGb * 10) / 10} GB`,
    participationLabel: connected
      ? peers.length === 0
        ? "No other devices yet"
        : `${peers.length} ${peers.length === 1 ? "peer" : "peers"} connected`
      : (snapshot?.sharingDeviceCount ?? 0) > 0
        ? // Someone else is sharing: the count is the reason to join.
          `${snapshot?.sharingDeviceCount ?? 0} sharing in this community`
        : // Cold. There is no status worth reporting, so the line explains what
          // the switch beside it actually does instead of restating a zero.
          "Share your spare compute with the Buzz community to run models",
    busyNow: (usage?.inflight ?? 0) > 0,
    activityLabel: describeActivity({ usage, isSharing, inboundWork }),
    originLabel: describeRequestOrigin(usage),
    modelCount: models.size,
  };
}
