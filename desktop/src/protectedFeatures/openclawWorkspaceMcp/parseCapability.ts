/** Hula-only OpenClaw workspace MCP capability (WSS / NOTICE payload). */

export const OPENCLAW_WORKSPACE_MCP_CAPABILITY_NAME = "openclaw.workspace_mcp";

export type OpenClawWorkspaceMcpCapability = {
  v: number;
  type: "hula.capability";
  name: typeof OPENCLAW_WORKSPACE_MCP_CAPABILITY_NAME;
  mcp: {
    url: string;
    transport?: string;
    authorization: string;
    expiresAt: string;
    /** Optional extra HTTP headers (e.g. CF Access). Authorization also lives here when present. */
    headers?: Record<string, string>;
  };
  hulaBuzzOnly?: boolean;
  relay?: string;
};

function parseHeaders(raw: unknown): Record<string, string> | undefined {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return undefined;
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof k !== "string" || !k.trim()) continue;
    if (typeof v !== "string" || !v.trim()) continue;
    out[k.trim()] = v.trim();
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

export function parseOpenClawWorkspaceMcpCapability(
  input: unknown,
): OpenClawWorkspaceMcpCapability | null {
  let value: unknown = input;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed.startsWith("{")) return null;
    try {
      value = JSON.parse(trimmed);
    } catch {
      return null;
    }
  }
  if (!value || typeof value !== "object") return null;
  const obj = value as Record<string, unknown>;
  if (obj.type !== "hula.capability") return null;
  if (obj.name !== OPENCLAW_WORKSPACE_MCP_CAPABILITY_NAME) return null;
  const mcp = obj.mcp;
  if (!mcp || typeof mcp !== "object") return null;
  const m = mcp as Record<string, unknown>;
  if (typeof m.url !== "string" || !m.url.trim()) return null;
  if (typeof m.authorization !== "string" || !m.authorization.trim())
    return null;
  if (typeof m.expiresAt !== "string" || !m.expiresAt.trim()) return null;
  const headers = parseHeaders(m.headers);
  const v = typeof obj.v === "number" ? obj.v : 1;
  return {
    v,
    type: "hula.capability",
    name: OPENCLAW_WORKSPACE_MCP_CAPABILITY_NAME,
    mcp: {
      url: m.url.trim(),
      transport: typeof m.transport === "string" ? m.transport : undefined,
      authorization: m.authorization.trim(),
      expiresAt: m.expiresAt.trim(),
      ...(headers ? { headers } : {}),
    },
    hulaBuzzOnly: obj.hulaBuzzOnly === true,
    relay: typeof obj.relay === "string" ? obj.relay : undefined,
  };
}
