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
   * A "shared" model is a layer-package (`meshllm/…-layers`) too big for one
   * machine. Serving it does nothing until enough members join and host it
   * together; the mesh then auto-splits it across the group.
   */
  shared: boolean;
  /**
   * For shared models: rough number of members like this machine needed to
   * host the model together. Advisory — the mesh decides the real split.
   */
  estimatedMembers: number | null;
};

export type MeshModelCatalog = {
  gpuName: string | null;
  vramDisplay: string;
  vramGb: number;
  recommended: string | null;
  /**
   * Ranked single-machine ("solo") models: recommended first, then by fit,
   * then larger first within a fit.
   */
  entries: MeshCatalogEntry[];
  /**
   * Curated shared/split models (`meshllm/…-layers`), smallest → largest.
   * Each is too big for one machine and runs split across several members.
   */
  shared: MeshCatalogEntry[];
};

/**
 * Hardware-aware curated model catalog for the Share-compute picker.
 * Works without a running mesh node (hardware survey + HF cache scan).
 */
export async function meshModelCatalog(): Promise<MeshModelCatalog> {
  return await invokeTauri<MeshModelCatalog>("mesh_model_catalog");
}
