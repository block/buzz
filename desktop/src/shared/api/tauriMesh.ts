import { invokeTauri } from "./tauri";

export type MeshHealth =
  | { status: "ok"; reason?: null }
  | { status: "degraded" | "failed"; reason: string };

export type MeshModelOption = {
  id: string;
  name: string | null;
};

export type MeshNodeState =
  | "off"
  | "starting"
  | "running"
  | "stopping"
  | "failed";
export type MeshNodeMode = "serve" | "client";

export type StartMeshNodeRequest = {
  mode: MeshNodeMode;
  modelId?: string;
  maxVramGb?: number;
  joinToken?: string;
};

export type MeshNodeStatus = {
  state: MeshNodeState;
  mode: MeshNodeMode | null;
  health: MeshHealth;
  apiBaseUrl: string | null;
  consoleUrl: string | null;
  modelId: string | null;
  modelName: string | null;
  inviteToken?: string | null;
  endpointId?: string | null;
  deviceId?: string | null;
  deviceName?: string | null;
};

export async function meshStartNode(
  request: StartMeshNodeRequest,
): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_start_node", { request });
}

export async function meshStopNode(): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_stop_node");
}

export async function meshNodeStatus(): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_node_status");
}

/**
 * This machine's own routing activity — **outbound**, not work done for others.
 *
 * Every counter here comes from mesh-llm's `routing_metrics`, which is
 * incremented only by the local OpenAI ingress (`network/openai/transport.rs`)
 * when *this* node dispatches a request. The inbound peer-serving path
 * (`mesh/stage_transport.rs`) never touches it — it only observes inflight.
 *
 * So the attempt split means where MY requests went, not who asked me:
 *   - `localAttempts`    — I ran it on my own GPU
 *   - `remoteAttempts`   — I sent it to a peer (i.e. I am CONSUMING)
 *   - `endpointAttempts` — I sent it to an endpoint
 *
 * `tokensServed` is likewise `completion_tokens_observed`: tokens I received,
 * not tokens I produced for someone else.
 *
 * mesh-llm exposes no inbound "requests I served for others" counter today, so
 * never label any field here as proof that this machine's compute was used by
 * another member. `inflight` is the one honest "busy right now" signal, and it
 * does not distinguish who the work is for.
 */
export type MeshServingUsage = {
  inflight: number;
  peakInflight: number;
  requestsServed: number;
  tokensServed: number;
  tokensPerSecond: number;
  localAttempts: number;
  remoteAttempts: number;
  endpointAttempts: number;
  peers: number;
  /**
   * Completed requests this machine's own GPU answered.
   *
   * From `routing_metrics.pressure`. Unlike the `*Attempts` counters these are
   * finished requests rather than tries, which makes them the honest basis for
   * "this ran here" vs "this ran on someone else's machine".
   */
  locallyServed: number;
  /**
   * Completed requests a peer answered for this machine — i.e. this machine
   * CONSUMED another member's compute. The one true "I used someone else's
   * hardware" figure available today.
   */
  remotelyServed: number;
  /** Completed requests answered by a configured endpoint, not a mesh peer. */
  endpointServed: number;
};

export async function meshServingUsage(): Promise<MeshServingUsage> {
  return await invokeTauri<MeshServingUsage>("mesh_serving_usage");
}

export async function meshInstalledModels(): Promise<MeshModelOption[]> {
  return await invokeTauri<MeshModelOption[]>("mesh_installed_models");
}

export type MeshModelFit = "comfortable" | "tight" | "tradeoff" | "too_large";

export type MeshCatalogEntry = {
  /** Catalog name — valid as-is in the model field. */
  name: string;
  /** Display size, e.g. "5.0GB". */
  size: string;
  sizeGb: number;
  description: string;
  fit: MeshModelFit;
  installed: boolean;
  recommended: boolean;
  /**
   * Buzz-curated pick — known to survive the agent harness. Curated entries
   * render above the fold; everything else is "advanced".
   */
  curated: boolean;
};

export type MeshModelCatalog = {
  gpuName: string | null;
  vramDisplay: string;
  vramGb: number;
  recommended: string | null;
  /** Ranked: recommended first, then curated, then by fit, larger first. */
  entries: MeshCatalogEntry[];
};

/**
 * Hardware-aware curated model catalog for the Share-compute picker.
 * Works without a running mesh node (hardware survey + HF cache scan).
 */
export async function meshModelCatalog(): Promise<MeshModelCatalog> {
  return await invokeTauri<MeshModelCatalog>("mesh_model_catalog");
}

/** Coarse per-device state. See Rust `MeshDeviceState`. */
export type MeshDeviceState = "serving" | "loading" | "standby" | "consuming";

export type MeshSnapshotDevice = {
  deviceId: string | null;
  label: string;
  /** Shared AI memory in GB, honoring that member's own cap. */
  capacityGb: number | null;
  models: string[];
  state: MeshDeviceState;
  isSelf: boolean;
  /** Additive Community View fields. Older desktop/runtime pairs omit these. */
  memberPubkey?: string | null;
  reportedAt?: number | null;
  modelSizeGb?: number | null;
};

/**
 * Community-wide shared-compute snapshot: who is sharing right now, how much
 * they are sharing, and what is ready to use.
 *
 * Presentation only — routing still goes through the availability path. An
 * empty mesh arrives as `reason` with zero counts, never as an error.
 */
export type MeshSnapshot = {
  sharingDeviceCount: number;
  /** Unique members with at least one ready serving device, when reported. */
  contributorMemberCount?: number;
  /** `null` when no sharing device reported a figure — render the count only. */
  sharedCapacityGb: number | null;
  /** Approximate hosted-model footprint when every contributor reports it. */
  allocatedCapacityGb?: number | null;
  models: string[];
  devices: MeshSnapshotDevice[];
  includesSelf: boolean;
  observedAt?: number;
  freshnessSeconds?: number;
  /**
   * Members in the community right now (NIP-43 roster size).
   *
   * The denominator for "N of M sharing". Never used to estimate unshared
   * capacity: a member who never starts a node publishes no status note, so
   * their hardware is unknown by design. Ghosts are a count, never GB.
   */
  memberCount: number;
  reason: string | null;
};

export async function meshSnapshot(): Promise<MeshSnapshot> {
  return await invokeTauri<MeshSnapshot>("mesh_snapshot");
}

/** What a peer is doing, as *it* reports. Never inferred from traffic. */
export type MeshPeerState = "serving" | "loading" | "standby" | "consuming";

/**
 * A node this machine's runtime is **actually connected to right now**.
 *
 * Distinct from `MeshSnapshotDevice`, which comes from relay status notes valid
 * for 120s and therefore outlives the node that published it. Peers come from
 * live gossip: present here means reachable now. See Rust `mesh_llm/peers.rs`.
 */
export type MeshPeer = {
  id: string;
  label: string;
  state: MeshPeerState;
  /** `null` when the peer shares none (client mode reports 0, shown as absent). */
  capacityGb: number | null;
  models: string[];
  rttMs: number | null;
};

export type MeshLiveView = {
  /**
   * True when a local runtime is up. Distinguishes "no peers" (alone on the
   * mesh) from "unknown" (not participating, nothing to report).
   */
  connected: boolean;
  selfCapacityGb: number | null;
  /** Approximate hosted-model footprint, not live OS GPU allocation. */
  selfModelSizeGb?: number | null;
  peers: MeshPeer[];
};

/**
 * This machine's live gossip view of the mesh. Empty and `connected:false` when
 * no node is running — a normal state, not an error.
 */
export async function meshLiveView(): Promise<MeshLiveView> {
  return await invokeTauri<MeshLiveView>("mesh_live_view");
}

/**
 * A model that can be served across several machines.
 *
 * Qualification is the presence of a published *layer package* — never a size
 * heuristic and never the repo name. A plain GGUF is one opaque blob; splitting
 * needs the repackaged form produced by `mesh-llm models package`, and mesh-llm
 * detects it by probing for the package manifest.
 */
export type MeshSplitEntry = {
  /** Model reference, valid as-is in the model field. */
  name: string;
  /** Catalog size label. Some entries publish "30 layers" instead of bytes. */
  sizeLabel: string | null;
  /** Parsed size, or null when the catalog publishes no byte size. */
  sizeGb: number | null;
  /** Layers in the package: the unit distributed across machines. */
  layerCount: number | null;
  /** The `-layers` repo holding the package. */
  packageRepo: string;
  /**
   * Whether this machine alone could hold it.
   *
   * Load-bearing: a split-capable model that fits locally is not an
   * experimental choice. Qwen3-8B is 5 GB and split-capable, and mesh-llm
   * returns `SingleNodeFit` for it before ever considering a split.
   */
  fitsLocally: boolean;
  description: string | null;
};

export type MeshExperimentalCatalog = {
  /** Split-capable models, smallest reach first. */
  entries: MeshSplitEntry[];
  /** This machine's usable AI memory — the figure `fitsLocally` is against. */
  vramGb: number;
  /**
   * The catalog cache is stale or unreadable. Surfaced rather than hidden: an
   * empty list means something different when the catalog could not be read
   * than when it genuinely holds no packages.
   */
  stale: boolean;
};

/**
 * Models that can be served across several machines.
 *
 * Deliberately node-independent — you choose what to share before you start
 * sharing it, so this works with the runtime off.
 */
export async function meshExperimentalCatalog(): Promise<MeshExperimentalCatalog> {
  return await invokeTauri<MeshExperimentalCatalog>(
    "mesh_experimental_catalog",
  );
}
