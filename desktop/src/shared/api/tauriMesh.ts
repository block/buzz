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

let meshDiagnosticLoggingEnabled = false;

export function meshDebugLog(message: string): void {
  if (!meshDiagnosticLoggingEnabled) return;
  void invokeTauri<void>("mesh_debug_log", { message }).catch(() => {
    // Diagnostic logging must never affect the UI path.
  });
}

export async function meshDebugLoggingEnabled(): Promise<boolean> {
  const enabled = await invokeTauri<boolean>("mesh_debug_logging_enabled");
  meshDiagnosticLoggingEnabled = enabled;
  return enabled;
}

export async function setMeshDebugLoggingEnabled(
  enabled: boolean,
): Promise<boolean> {
  const saved = await invokeTauri<boolean>("set_mesh_debug_logging_enabled", {
    enabled,
  });
  meshDiagnosticLoggingEnabled = saved;
  return saved;
}

export async function meshStartNode(
  request: StartMeshNodeRequest,
): Promise<MeshNodeStatus> {
  meshDebugLog(`api mesh_start_node invoke ${JSON.stringify(request)}`);
  try {
    const status = await invokeTauri<MeshNodeStatus>("mesh_start_node", {
      request,
    });
    meshDebugLog(
      `api mesh_start_node ok state=${status.state} mode=${status.mode} model=${status.modelId}`,
    );
    return status;
  } catch (err) {
    meshDebugLog(
      `api mesh_start_node error ${err instanceof Error ? err.message : String(err)}`,
    );
    throw err;
  }
}

export async function meshStopNode(): Promise<MeshNodeStatus> {
  meshDebugLog("api mesh_stop_node invoke");
  try {
    const status = await invokeTauri<MeshNodeStatus>("mesh_stop_node");
    meshDebugLog(
      `api mesh_stop_node ok state=${status.state} mode=${status.mode} model=${status.modelId}`,
    );
    return status;
  } catch (err) {
    meshDebugLog(
      `api mesh_stop_node error ${err instanceof Error ? err.message : String(err)}`,
    );
    throw err;
  }
}

export async function meshNodeStatus(): Promise<MeshNodeStatus> {
  meshDebugLog("api mesh_node_status invoke");
  try {
    const status = await invokeTauri<MeshNodeStatus>("mesh_node_status");
    meshDebugLog(
      `api mesh_node_status ok state=${status.state} mode=${status.mode} model=${status.modelId}`,
    );
    return status;
  } catch (err) {
    meshDebugLog(
      `api mesh_node_status error ${err instanceof Error ? err.message : String(err)}`,
    );
    throw err;
  }
}

/**
 * Host-side usage of the compute this machine is sharing. The
 * local/remote/endpoint attempt split distinguishes this machine's own agents
 * (local) from another member consuming this machine's compute (remote/endpoint).
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
};

export async function meshServingUsage(): Promise<MeshServingUsage> {
  meshDebugLog("api mesh_serving_usage invoke");
  try {
    const usage = await invokeTauri<MeshServingUsage>("mesh_serving_usage");
    meshDebugLog(
      `api mesh_serving_usage ok inflight=${usage.inflight} requests=${usage.requestsServed} remote=${usage.remoteAttempts}`,
    );
    return usage;
  } catch (err) {
    meshDebugLog(
      `api mesh_serving_usage error ${err instanceof Error ? err.message : String(err)}`,
    );
    throw err;
  }
}

export async function meshInstalledModels(): Promise<MeshModelOption[]> {
  meshDebugLog("api mesh_installed_models invoke");
  try {
    const models = await invokeTauri<MeshModelOption[]>(
      "mesh_installed_models",
    );
    meshDebugLog(`api mesh_installed_models ok count=${models.length}`);
    return models;
  } catch (err) {
    meshDebugLog(
      `api mesh_installed_models error ${err instanceof Error ? err.message : String(err)}`,
    );
    throw err;
  }
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
  meshDebugLog("api mesh_model_catalog invoke");
  try {
    const catalog = await invokeTauri<MeshModelCatalog>("mesh_model_catalog");
    meshDebugLog(
      `api mesh_model_catalog ok entries=${catalog.entries.length} recommended=${catalog.recommended} vram=${catalog.vramGb}`,
    );
    return catalog;
  } catch (err) {
    meshDebugLog(
      `api mesh_model_catalog error ${err instanceof Error ? err.message : String(err)}`,
    );
    throw err;
  }
}
