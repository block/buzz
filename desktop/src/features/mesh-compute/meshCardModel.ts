import type {
  MeshLiveView,
  MeshNodeStatus,
  MeshServingUsage,
  MeshSnapshot,
  MeshSnapshotDevice,
} from "@/shared/api/tauriMesh";
import type { MeshDownloadProgress } from "./hooks/useMeshDownloadProgress";
import { describeParticipationHint } from "./meshActivity";
import type { MeshShareToggleModel } from "./shareToggleState";

/**
 * Pure projection for the sidebar shared-compute card.
 *
 * The card has one job: make it obvious, at a glance, whether **this machine**
 * is giving compute, taking compute, or neither — and what the community has
 * to offer. Copy lives here (not in the component) so the wording is unit
 * tested and cannot drift between states.
 *
 * Two distinctions this model refuses to blur:
 *
 * 1. **Sharing vs consuming.** One mesh runtime slot serves both roles and both
 *    report `state:"running"`, so the role comes from `deriveMeshShareToggle`,
 *    never from `state` alone. Consuming must never read as sharing.
 * 2. **Unknown capacity vs zero capacity.** `sharedCapacityGb: null` means
 *    nobody reported a figure, so the headline drops the number rather than
 *    claiming "0 GB".
 */

/** Which role this machine is playing. Drives the card's accent + icon. */
export type MeshCardTone =
  | "idle"
  | "sharing"
  | "consuming"
  | "pending"
  | "failed";

export type MeshCardModel = {
  tone: MeshCardTone;
  /** Primary line — what is happening, from this machine's point of view. */
  headline: string;
  /** Secondary line — the community context, or the reason for a problem. */
  detail: string | null;
  /** Switch position. Reflects serve-mode occupancy only. */
  switchOn: boolean;
  switchDisabled: boolean;
  /** Accessible label for the switch, naming the consequence of flipping it. */
  switchLabel: string;
  /** Devices for the topology strip, sharing first. */
  devices: MeshSnapshotDevice[];
  /** True when the mesh has exactly one participant (the solo case). */
  isSolo: boolean;
  /** Whether to show the "waiting for another device" hint. */
  showSoloHint: boolean;
};

/**
 * Format shared memory for display. Whole numbers at 10GB+ (a "42 GB" mesh
 * reads better than "42.3 GB"); one decimal below that so a single small
 * machine still shows a meaningful figure.
 */
export function formatCapacityGb(gb: number): string {
  const rounded = gb >= 10 ? Math.round(gb) : Math.round(gb * 10) / 10;
  return `${rounded} GB`;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return n === 1 ? one : many;
}

/**
 * The community headline: how much compute is actually reachable right now.
 *
 * Named "Mesh capacity" rather than "sharing" because the figure describes the
 * pool, not this machine's participation — the switch already says whether you
 * are in it.
 *
 * Counts only devices advertising a routable model — the same standard routing
 * uses — so the number never promises capacity that cannot be reached. Never
 * says "total": the underlying query is capped at 100 members.
 */
export function describeMeshCapacity(snapshot: MeshSnapshot | null): string {
  // `null` is "not fetched yet", NOT "the community has nothing". Claiming an
  // empty mesh before the first snapshot lands reads as a verdict on everyone
  // else's machines while the card is still starting up.
  if (!snapshot) {
    return "Checking mesh capacity…";
  }
  if (snapshot.sharingDeviceCount === 0) {
    return "No mesh capacity yet";
  }
  const { sharingDeviceCount: count, sharedCapacityGb: gb } = snapshot;
  const devices = `${count} ${plural(count, "device")}`;
  // Unknown capacity degrades to the device count rather than printing 0 GB.
  return gb === null
    ? `Mesh capacity · ${devices}`
    : `${formatCapacityGb(gb)} · ${devices}`;
}

/**
 * The card headline once we have a live view: name the mesh, its capacity, and
 * how many nodes we can actually see.
 *
 * Prefers the live gossip view over the relay snapshot. Peers listed here are
 * ones our runtime is talking to *now*, whereas relay status notes stay valid
 * for 120s and so outlive the node that wrote them — a graph built on notes
 * shows devices gossip already knows are gone.
 *
 * Capacity sums this machine plus its serving peers. Consuming peers contribute
 * a peer count but no GB, because they share none.
 */
export function describeMeshHeadline({
  view,
  snapshot,
}: {
  view: MeshLiveView | null;
  snapshot: MeshSnapshot | null;
}): string {
  // Not participating: the relay snapshot is the only view of the pool, and the
  // reason to consider joining.
  if (!view?.connected) {
    return describeMeshCapacity(snapshot);
  }
  const capacityGb = [
    view.selfCapacityGb ?? 0,
    ...view.peers.map((peer) => peer.capacityGb ?? 0),
  ].reduce((total, gb) => total + gb, 0);
  const peerCount = view.peers.length;
  const peers = `${peerCount} ${plural(peerCount, "peer")}`;
  return capacityGb > 0
    ? `MeshLLM · ${formatCapacityGb(capacityGb)}, ${peers}`
    : `MeshLLM · ${peers}`;
}

/** Short label for what is ready to run, or null when nothing is. */
export function describeReadyModels(
  snapshot: MeshSnapshot | null,
): string | null {
  const models = snapshot?.models ?? [];
  if (models.length === 0) {
    return null;
  }
  if (models.length === 1) {
    return `${shortModelLabel(models[0])} ready`;
  }
  return `${models.length} models ready`;
}

/**
 * Name the startup stage instead of showing one opaque "Starting…".
 *
 * A first-time start does three very different things behind one spinner:
 * resolve the model, download several gigabytes, then load it into memory. The
 * download dominates — minutes, not seconds — and an unlabelled spinner during
 * it reads as a hang. So the download is named and measured; everything else
 * stays honest about being indeterminate.
 */
export function describeStartupHeadline(
  progress: MeshDownloadProgress | null,
): string {
  if (progress?.status === "downloading") {
    const pct = downloadPercent(progress);
    return pct === null ? "Downloading model…" : `Downloading model · ${pct}%`;
  }
  if (progress?.status === "preparing") {
    return "Preparing model…";
  }
  return "Starting to share…";
}

/** Secondary line for the startup stages. */
export function describeStartupStage(
  progress: MeshDownloadProgress | null,
  modelRef?: string | null,
): string {
  const modelLabel = shortModelLabel(progress?.label || modelRef || "");
  const modelPrefix = modelLabel ? `${modelLabel} · ` : "";
  if (progress?.status === "downloading") {
    const total = progress.totalBytes;
    return total === null
      ? `${modelPrefix}First run downloads the model once.`
      : `${modelPrefix}${formatBytes(progress.downloadedBytes ?? 0)} of ${formatBytes(total)} · first run only`;
  }
  if (progress?.status === "preparing") {
    return `${modelPrefix}Checking what's already downloaded.`;
  }
  return modelLabel
    ? `Loading ${modelLabel} into memory.`
    : "Loading the model into memory.";
}

function downloadPercent(progress: MeshDownloadProgress): number | null {
  const { downloadedBytes, totalBytes } = progress;
  // A percentage needs both figures and a non-zero denominator; without them a
  // bare "Downloading…" beats a fabricated 0%.
  if (downloadedBytes === null || totalBytes === null || totalBytes <= 0) {
    return null;
  }
  return Math.min(100, Math.floor((downloadedBytes / totalBytes) * 100));
}

function formatBytes(bytes: number): string {
  const gb = bytes / 1e9;
  if (gb >= 1) {
    return `${Math.round(gb * 10) / 10} GB`;
  }
  return `${Math.round(bytes / 1e6)} MB`;
}

/**
 * Trim a model reference down to something that fits a 256px sidebar.
 * `unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M` → `Gemma 4 26B A4B`.
 */
export function shortModelLabel(modelRef: string): string {
  const basename = modelRef.split("/").at(-1) ?? modelRef;
  const withoutQuant = basename.replace(/[:-](?:GGUF|UD)?[:-]?Q\d.*$/i, "");
  const cleaned = withoutQuant
    .replace(/-GGUF.*$/i, "")
    .replace(/-it$/i, "")
    .replaceAll("-", " ")
    .trim();
  if (cleaned === "") {
    return basename;
  }
  return cleaned.charAt(0).toUpperCase() + cleaned.slice(1);
}

/**
 * Project everything the card needs into one view model.
 *
 * Total: every input may be null (nothing fetched yet) and the result is still
 * renderable.
 */
export function deriveMeshCardModel({
  snapshot,
  status,
  toggle,
  pendingAction,
  canShare,
  view,
  usage,
  inboundWork,
  downloadProgress,
  startingModel,
}: {
  snapshot: MeshSnapshot | null;
  status: MeshNodeStatus | null;
  toggle: MeshShareToggleModel;
  pendingAction: "start" | "stop" | null;
  /** False when no model can be resolved yet (catalog still loading). */
  canShare: boolean;
  /** Live gossip view. Null/disconnected falls back to the relay snapshot. */
  view: MeshLiveView | null;
  /** This node's own routing counters. All outbound — see `meshActivity.ts`. */
  usage: MeshServingUsage | null;
  /**
   * Inbound work inferred by elimination (serving + inflight + our own dispatch
   * count flat). Sampled, so it can undercount; it never over-claims.
   */
  inboundWork: boolean;
  /**
   * Live model-download progress, when a download is running.
   *
   * Without this, a first-time start shows "Starting to share…" for however
   * long a multi-gigabyte download takes, which reads as a hang. A download is
   * the single longest and most opaque step, so it gets named and measured.
   */
  downloadProgress: MeshDownloadProgress | null;
  /** Model requested by the current start action before node status exists. */
  startingModel?: string | null;
}): MeshCardModel {
  const devices = snapshot?.devices ?? [];
  const headline = describeMeshHeadline({ view, snapshot });
  const ready = describeReadyModels(snapshot);
  // Solo means "connected but nobody else is here" — a live-view fact. The
  // relay snapshot cannot tell us this: a lone note may just be a stale one.
  const isSolo = view?.connected === true && view.peers.length === 0;
  const startingDetail = describeStartupStage(
    downloadProgress,
    status?.modelName ?? status?.modelId ?? startingModel,
  );
  const hint = describeParticipationHint({
    isSharing: toggle.isSharing,
    isConsuming: toggle.isConsuming,
    inboundWork,
    usage,
  });

  const base = {
    devices,
    isSolo,
    switchOn: toggle.isSharing,
    // A serve node can always be stopped. Anything else waits for a resolvable
    // model, and never lets an unknown occupant be replaced silently.
    switchDisabled:
      pendingAction !== null ||
      (toggle.isSharing
        ? false
        : toggle.slotOccupied && !toggle.isConsuming
          ? true
          : !canShare),
    switchLabel: toggle.isSharing
      ? "Stop sharing this computer's compute"
      : "Share this computer's compute",
  };

  if (pendingAction === "start") {
    return {
      ...base,
      tone: "pending",
      headline: describeStartupHeadline(downloadProgress),
      detail: startingDetail,
      showSoloHint: false,
    };
  }
  if (pendingAction === "stop") {
    return {
      ...base,
      tone: "pending",
      headline: "Stopping…",
      detail: null,
      showSoloHint: false,
    };
  }

  // Consuming: this machine is TAKING compute, not giving it. Say so plainly —
  // the switch is off here and that must not read as "nothing is happening".
  if (toggle.isConsuming) {
    return {
      ...base,
      tone: "consuming",
      headline,
      detail: hint,
      showSoloHint: false,
    };
  }

  if (toggle.isSharing) {
    const health = status?.health;
    if (health && health.status !== "ok") {
      return {
        ...base,
        tone: "failed",
        headline: "Sharing needs attention",
        detail:
          health.reason ?? "The shared compute runtime reported a problem.",
        showSoloHint: false,
      };
    }
    // A serve node with no advertised model yet is warming up, not serving.
    const selfDevice = devices.find((device) => device.isSelf);
    if (selfDevice?.state === "loading" || status?.state === "starting") {
      return {
        ...base,
        tone: "pending",
        headline: describeStartupHeadline(downloadProgress),
        detail: startingDetail,
        showSoloHint: false,
      };
    }
    const activeModel = status?.modelName ?? status?.modelId;
    return {
      ...base,
      tone: "sharing",
      headline: "You’re sharing compute.",
      detail: activeModel ? shortModelLabel(activeModel) : "Model ready",
      showSoloHint: false,
    };
  }

  // Idle. Two very different situations share this branch, and conflating them
  // was the old bug: a community with compute to offer is an invitation, while
  // an empty one is a call to be first. Neither is an error.
  const communityIsEmpty =
    snapshot !== null && snapshot.sharingDeviceCount === 0;
  if (communityIsEmpty) {
    return {
      ...base,
      tone: "idle",
      headline,
      // No hedging about what *might* be available: nobody is sharing, so the
      // only true statement is that turning this on creates the capacity.
      detail: canShare
        ? "Be the first to share compute here."
        : // A machine that cannot host anything is not broken, and saying
          // "turn it on" to someone who cannot is a dead end. It can still
          // consume once somebody else shares.
          "This computer is too small to share, but can use shared compute.",
      showSoloHint: false,
    };
  }

  // The community has compute. Lead with what it already has, because that is
  // the reason to join — not with a description of the mechanism.
  return {
    ...base,
    tone: "idle",
    headline,
    detail: ready ?? hint,
    showSoloHint: false,
  };
}
