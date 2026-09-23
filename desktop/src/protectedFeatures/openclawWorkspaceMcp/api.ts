import { invokeTauri } from "@/shared/api/tauri";

export type OpenClawWorkspaceStatus = {
  connected: boolean;
  expiresAt?: string | null;
  url?: string | null;
  connectedViaRelay: boolean;
  agentsUpdated: number;
};

export type OpenClawWorkspaceTestResult = {
  ok: boolean;
  message: string;
  httpStatus?: number | null;
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

/** Re-apply stored grant to Claude MCP configs (does not mint a new JWT). */
export async function refreshOpenClawWorkspace(): Promise<OpenClawWorkspaceStatus> {
  return invokeTauri<OpenClawWorkspaceStatus>("refresh_openclaw_workspace_mcp");
}

/** Authenticated MCP initialize ping using the stored grant. */
export async function testOpenClawWorkspace(): Promise<OpenClawWorkspaceTestResult> {
  return invokeTauri<OpenClawWorkspaceTestResult>(
    "test_openclaw_workspace_mcp",
  );
}

/** Notify UI (create Switch gate, etc.) that OpenClaw workspace status changed. */
export function emitOpenClawWorkspaceStatusChanged(): void {
  window.dispatchEvent(new Event("buzz:openclaw-workspace-status"));
}
