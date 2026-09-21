import { invokeTauri } from "@/shared/api/tauri";

export type OpenClawWorkspaceStatus = {
  connected: boolean;
  expiresAt?: string | null;
  url?: string | null;
  connectedViaRelay: boolean;
  agentsUpdated: number;
};

export async function fetchOpenClawWorkspaceStatus(): Promise<OpenClawWorkspaceStatus> {
  return invokeTauri<OpenClawWorkspaceStatus>(
    "get_openclaw_workspace_mcp_status",
  );
}

export async function applyOpenClawWorkspaceGrant(input: {
  url: string;
  authorization: string;
  expiresAt: string;
  headers?: Record<string, string>;
  relay?: string;
  connectedViaRelay?: boolean;
}): Promise<OpenClawWorkspaceStatus> {
  return invokeTauri<OpenClawWorkspaceStatus>(
    "apply_openclaw_workspace_mcp_grant",
    {
      url: input.url,
      authorization: input.authorization,
      expiresAt: input.expiresAt,
      headers: input.headers ?? null,
      relay: input.relay ?? null,
      connectedViaRelay: input.connectedViaRelay ?? true,
    },
  );
}

export async function disconnectOpenClawWorkspace(): Promise<OpenClawWorkspaceStatus> {
  return invokeTauri<OpenClawWorkspaceStatus>(
    "disconnect_openclaw_workspace_mcp",
  );
}
