import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";

import { invokeTauri } from "@/shared/api/tauri";

export type McpAppTool = {
  name: string;
  title: string | null;
  description: string | null;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown> | null;
  annotations: Record<string, unknown> | null;
  meta: Record<string, unknown>;
  uiResourceUri: string | null;
  visibility: Array<"model" | "app">;
};

export type McpAppResource = {
  uri: string;
  name: string | null;
  title: string | null;
  description: string | null;
  mimeType: string | null;
  meta: Record<string, unknown>;
};

export type McpAppServerDescriptor = {
  serverId: string;
  endpoint: string;
  name: string;
  version: string | null;
  protocolVersion: string;
  tools: McpAppTool[];
  resources: McpAppResource[];
};

export type McpAppResourceCsp = {
  connectDomains?: string[];
  resourceDomains?: string[];
  frameDomains?: string[];
  baseUriDomains?: string[];
};

export type McpAppResourcePermissions = {
  camera?: Record<string, never>;
  microphone?: Record<string, never>;
  geolocation?: Record<string, never>;
  clipboardWrite?: Record<string, never>;
};

export type PreparedMcpAppView = {
  viewId: string;
  sandboxUrl: string;
  html: string;
  csp: McpAppResourceCsp;
  requestedPermissions: McpAppResourcePermissions;
};

export async function connectMcpAppServer(
  endpoint: string,
): Promise<McpAppServerDescriptor> {
  return invokeTauri("connect_mcp_app_server", { endpoint });
}

export async function listMcpAppTools(serverId: string): Promise<McpAppTool[]> {
  return invokeTauri("list_mcp_app_tools", { serverId });
}

export async function listMcpAppResources(
  serverId: string,
): Promise<McpAppResource[]> {
  return invokeTauri("list_mcp_app_resources", { serverId });
}

export async function callMcpAppTool(
  serverId: string,
  name: string,
  args: Record<string, unknown>,
  caller: "host" | "app",
): Promise<CallToolResult> {
  return invokeTauri("call_mcp_app_tool", {
    serverId,
    name,
    arguments: args,
    caller,
  });
}

export async function readMcpAppResource(
  serverId: string,
  uri: string,
): Promise<Record<string, unknown>> {
  return invokeTauri("read_mcp_app_resource", { serverId, uri });
}

export async function prepareMcpAppView(
  serverId: string,
  uri: string,
): Promise<PreparedMcpAppView> {
  return invokeTauri("prepare_mcp_app_view", { serverId, uri });
}

export async function releaseMcpAppView(viewId: string): Promise<void> {
  await invokeTauri("release_mcp_app_view", { viewId });
}

export async function disconnectMcpAppServer(serverId: string): Promise<void> {
  await invokeTauri("disconnect_mcp_app_server", { serverId });
}
