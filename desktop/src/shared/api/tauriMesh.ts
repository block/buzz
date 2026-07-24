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

export async function meshInstalledModels(): Promise<MeshModelOption[]> {
  return await invokeTauri<MeshModelOption[]>("mesh_installed_models");
}

export type MeshServeTarget = {
  modelId: string;
  modelName: string | null;
  endpointAddr: string;
  nodeName: string | null;
  capacity: { vramGb: number | null } | null;
  endpointId?: string | null;
  deviceId?: string | null;
  deviceName?: string | null;
};

export type MeshAvailability = {
  reason: string | null;
  models: MeshModelOption[];
  serveTargets: MeshServeTarget[];
};

/**
 * Live Buzz shared compute availability on this relay: member-verified,
 * freshness-filtered serve targets and the models they host. Read-only — used
 * to tell the user where relay-mesh inference actually runs.
 */
export async function meshAvailability(): Promise<MeshAvailability> {
  return await invokeTauri<MeshAvailability>("mesh_availability");
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
