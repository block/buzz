import { invokeTauri } from "./tauri";

export type MeshHealth =
  | { status: "ok"; reason?: null }
  | { status: "degraded" | "failed"; reason: string };

export type MeshModelOption = {
  id: string;
  name: string | null;
};

export type MeshServeTarget = {
  modelId: string;
  modelName: string | null;
  endpointAddr: string;
  nodeName: string | null;
  capacity: { vramGb: number | null } | null;
  reporterPubkey?: string | null;
  endpointId?: string | null;
  deviceId?: string | null;
  deviceName?: string | null;
};

export type MeshAvailability = {
  capable: boolean;
  admitted: boolean;
  available: boolean;
  reason: string | null;
  models: MeshModelOption[];
  serveTargets: MeshServeTarget[];
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

export type MeshCallMeNow = {
  v: 1;
  type: "buzz-iroh-call-me-now";
  peer_endpoint_addr: string;
  peer_endpoint_id?: string;
  attempt_id: string;
  expires_at: number;
};

export type MeshAgentPreset = {
  providerId: "relay-mesh";
  label: string;
  acpCommand: string;
  agentCommand: string;
  agentArgs: string[];
  mcpCommand: string;
  model: string;
  envVars: Record<string, string>;
};

export async function meshAvailability(): Promise<MeshAvailability> {
  return await invokeTauri<MeshAvailability>("mesh_availability");
}

export async function meshStartNode(
  request: StartMeshNodeRequest,
): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_start_node", { request });
}

export async function meshEnsureClientNode(
  modelId: string,
  target?: MeshServeTarget | null,
): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_ensure_client_node", {
    request: {
      modelId,
      endpointAddr: target?.endpointAddr,
      reporterPubkey: target?.reporterPubkey,
      peerEndpointId: target?.endpointId,
    },
  });
}

export async function meshPrepareRelayMeshClient(
  modelId: string,
  target: MeshServeTarget,
): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_prepare_relay_mesh_client", {
    request: { modelId, target },
  });
}

export async function meshDialEndpointAddr(
  endpointAddr: string,
): Promise<MeshNodeStatus> {
  return await invokeTauri<MeshNodeStatus>("mesh_dial_endpoint_addr", {
    request: { endpointAddr },
  });
}

export async function meshStatusReportPayload(): Promise<Record<
  string,
  unknown
> | null> {
  return await invokeTauri<Record<string, unknown> | null>(
    "mesh_status_report_payload",
  );
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

export async function meshAgentPreset(
  modelId: string,
): Promise<MeshAgentPreset> {
  return await invokeTauri<MeshAgentPreset>("mesh_agent_preset", {
    request: { modelId },
  });
}
