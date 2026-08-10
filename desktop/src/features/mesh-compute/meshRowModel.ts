import type {
  MeshLiveView,
  MeshNodeStatus,
  MeshSnapshot,
} from "@/shared/api/tauriMesh";
import type { MeshShareToggleModel } from "./shareToggleState";
import { formatCapacityGb, plural } from "./meshCardModel";

/**
 * Pure projection for the sidebar Community mesh row.
 *
 * The row replaced a ~120px card with a single menu line, so it carries two
 * signals only: a coloured dot and a capacity figure. Every control lives in
 * the popover.
 *
 * ## Why "not participating" is two states, not one
 *
 * Grey would conflate "nobody is sharing anything" with "there is 115 GB here
 * and you simply have not joined" — and the second is the whole reason to look
 * at this row. They are distinguishable without any probe: `mesh_snapshot`
 * reads other members' relay status notes and needs no local runtime, so we
 * know the pool exists while our own node is off.
 *
 * `unknown` therefore means *no capacity known* (nothing published, or not
 * fetched yet), and `available` means *capacity exists and we are outside it*.
 */
export type MeshRowTone =
  /** Nothing published by anyone, or the snapshot has not landed yet. */
  | "unknown"
  /** The community has capacity; this machine is not in the mesh. */
  | "available"
  /** Serving: this machine contributes compute. */
  | "sharing"
  /** Client only: this machine uses someone else's compute. */
  | "consuming"
  /** A start/stop is in flight, or the node is loading a model. */
  | "starting"
  /** The runtime occupies the slot but reports unhealthy. */
  | "failed";

/**
 * Why the dot is moving, not merely whether.
 *
 * **Motion means something is happening.** Only two facts qualify:
 *
 *   - `activity` — work is in flight on this node right now
 *   - `progress` — a start or stop is in flight
 *
 * Everything else is a *standing state* and must not animate. An earlier
 * revision pulsed an "act here" invitation — capacity you have not joined, or
 * sharing with no agent using it — and because those conditions are permanently
 * true they produced a dot that moved forever beside a perfectly healthy row.
 * Constant motion is not a signal; it is noise that trains people to ignore the
 * indicator. Standing states belong in the tone, the tooltip, and the popover's
 * own prompt, all of which already carry them.
 *
 * Both kinds animate brightness only, on the dot itself. No scaling: a growing
 * dot shifts the row's optical baseline and reads as jitter. Opacity is what
 * Tailwind's `pulse` animates and what Buzz already uses to mean "working"
 * (`ChannelWorkingBadge`, `AgentStatusBadge`). They differ in tempo, not
 * character — 1s for happening-now, 1.8s for in-progress.
 */
export type MeshRowPulse = "none" | "activity" | "progress";

export type MeshRowModel = {
  tone: MeshRowTone;
  /** Row label. */
  label: string;
  /** Accessible state sentence, used as the row tooltip. */
  tooltip: string;
  /**
   * Compact trailing figure, or null when no capacity is known anywhere.
   *
   * Shown in every state that has a number, not just while sharing. When this
   * machine is outside the mesh the figure *is* the invitation: a real number
   * argues for joining better than any exhortation, and unlike a nudge there is
   * nothing to dismiss.
   *
   * When the mesh is empty there is no figure, so the slot carries the word
   * "Share" — the row would otherwise be inert with nothing explaining why.
   */
  badge: string | null;
  /**
   * Invitation text, or null.
   *
   * Only set in the cold state — no capacity anywhere, nothing running. There
   * is no number to show and no status to report, so without words the row is
   * inert and unexplained. Every other state has a figure or a tone that speaks
   * for itself, and a permanent nudge beside them would become furniture.
   */
  callToAction: string | null;
  /**
   * Why the dot moves, if it does. Never decorative: a still dot is a real
   * statement that there is nothing to act on and nothing happening.
   */
  pulse: MeshRowPulse;
};

/** One name in every state. */
const LABEL = "Shared Compute";

/** Live pool capacity: this machine plus every peer that reports a figure. */
function liveCapacityGb(view: MeshLiveView | null): number {
  if (!view?.connected) return 0;
  return [
    view.selfCapacityGb ?? 0,
    ...view.peers.map((peer) => peer.capacityGb ?? 0),
  ].reduce((total, gb) => total + gb, 0);
}

/**
 * Which pulse a participating node shows.
 *
 * Only real work moves the dot. `recentActivity` is not a second kind of signal:
 * `inflight` is sampled every few seconds, so a request that starts and finishes
 * between two polls is never seen at all. Latching the last sighting stops us
 * forgetting observed work immediately — it never invents any.
 */
function pulseForWork({
  busyNow,
  recentActivity,
}: {
  busyNow: boolean;
  recentActivity: boolean;
}): MeshRowPulse {
  return busyNow || recentActivity ? "activity" : "none";
}

export function deriveMeshRowModel({
  snapshot,
  status,
  toggle,
  view,
  pendingAction,
  busyNow,
  recentActivity = false,
  hasMeshAgent,
}: {
  snapshot: MeshSnapshot | null;
  status: MeshNodeStatus | null;
  toggle: MeshShareToggleModel;
  view: MeshLiveView | null;
  pendingAction: "start" | "stop" | null;
  /** Work in flight on this node, inbound or outbound. */
  busyNow: boolean;
  /**
   * Work was in flight within the last few seconds.
   *
   * `busyNow` alone under-reports badly: `inflight` is sampled every 4s, so any
   * request that starts and finishes between two polls is never seen and the dot
   * never moves. Latching the last sighting makes real short work visible
   * instead of silently dropping it. Still a fact about observed work — it never
   * invents activity, it only stops forgetting it immediately.
   */
  recentActivity?: boolean;
  /**
   * Any agent resolves to the shared-compute provider.
   *
   * `undefined` while the agent list is loading — treated as "assume set up",
   * so a slow query never throbs a nudge that then vanishes.
   */
  hasMeshAgent: boolean | undefined;
}): MeshRowModel {
  // A start/stop in flight outranks everything: the tone must not claim a
  // steady state while the runtime is mid-transition.
  if (
    pendingAction !== null ||
    (toggle.isSharing && status?.state === "starting")
  ) {
    return {
      tone: "starting",
      label: LABEL,
      tooltip:
        pendingAction === "stop" ? "Stopping sharing…" : "Starting to share…",
      badge: null,
      callToAction: null,
      // Loading is exactly when motion earns its keep. `meshStartNode` does not
      // resolve until the runtime is serving, which on a 17 GB model is minutes;
      // a still amber dot for that whole stretch reads as stuck rather than
      // working. `progress` is patient rather than insistent, because nothing is
      // wrong and nothing needs a decision.
      pulse: "progress",
    };
  }

  if (toggle.isSharing) {
    const health = status?.health;
    if (health && health.status !== "ok") {
      return {
        tone: "failed",
        label: LABEL,
        tooltip: "Sharing failed — open for details",
        badge: null,
        callToAction: null,
        pulse: "none",
      };
    }
    const gb = liveCapacityGb(view);
    const peerCount = view?.peers.length ?? 0;
    const badge = gb > 0 ? formatCapacityGb(gb) : null;
    // Sharing compute nothing can use is a dead end, and the row is where it
    // would go unnoticed. Green stays — we really are sharing — but the throb
    // says there is a step left, and only once the agent list has actually
    // resolved.
    if (hasMeshAgent === false) {
      return {
        tone: "sharing",
        label: LABEL,
        tooltip: "Sharing compute · no agent is using it yet",
        badge,
        callToAction: null,
        // Standing state, so it does not animate. This condition stays true for
        // as long as no agent uses the mesh, and a green dot pulsing forever
        // beside a healthy row is noise. The tooltip says it, and the popover
        // offers the fix.
        pulse: "none",
      };
    }
    return {
      tone: "sharing",
      label: LABEL,
      tooltip:
        peerCount > 0
          ? `Sharing compute · ${peerCount} ${plural(peerCount, "peer")}`
          : "Sharing compute · waiting for another device",
      badge,
      callToAction: null,
      pulse: pulseForWork({ busyNow, recentActivity }),
    };
  }

  if (toggle.isConsuming) {
    const gb = liveCapacityGb(view);
    return {
      tone: "consuming",
      label: LABEL,
      tooltip: "Using shared compute from the mesh",
      badge: gb > 0 ? formatCapacityGb(gb) : null,
      callToAction: null,
      pulse: pulseForWork({ busyNow, recentActivity }),
    };
  }

  // Not participating. The relay snapshot is the only view, and the only thing
  // that distinguishes "there is a mesh to join" from "there is nothing here".
  const count = snapshot?.sharingDeviceCount ?? 0;
  if (!snapshot) {
    // Not asked yet. No figure, no prompt: a call to action based on no data
    // would appear for one poll and retract, which reads as a glitch.
    return {
      tone: "unknown",
      label: LABEL,
      tooltip: "Checking for shared compute…",
      badge: null,
      // Nothing offered from no data: a prompt that appears for one poll and
      // retracts reads as a glitch, not a suggestion.
      callToAction: null,
      pulse: "none",
    };
  }
  if (count === 0) {
    // An empty mesh is the one state with nothing to show and nothing
    // happening, so without a prompt the row is inert and unexplained. The
    // action is real — you can be the first to share — so the badge slot
    // carries the word instead of a figure.
    //
    // Static, not throbbing. A community that never uses the mesh would
    // otherwise animate forever, and permanent motion earns nothing but
    // annoyance. The invite throb stays reserved for capacity that exists.
    return {
      tone: "unknown",
      label: LABEL,
      tooltip: "No shared compute yet — share this computer to start the mesh",
      badge: null,
      callToAction: "Share your compute",
      pulse: "none",
    };
  }
  const gb = snapshot.sharedCapacityGb;
  return {
    tone: "available",
    label: LABEL,
    tooltip:
      gb === null
        ? `Shared compute available · ${count} ${plural(count, "device")}`
        : `${formatCapacityGb(gb)} of shared compute available`,
    // Unknown capacity degrades to a device count rather than printing 0 GB — a
    // count still says "there is something here", which is the job.
    badge:
      gb === null
        ? `${count} ${plural(count, "device")}`
        : formatCapacityGb(gb),
    callToAction: null,
    // Standing state: this capacity sits there whether or not anyone looks. The
    // slate tone and the capacity figure in the tooltip are the invitation.
    pulse: "none",
  };
}
