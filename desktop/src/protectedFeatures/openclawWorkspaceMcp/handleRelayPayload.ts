import { applyOpenClawWorkspaceGrant } from "./api";
import { parseOpenClawWorkspaceMcpCapability } from "./parseCapability";

/** Attempt to parse + apply a Hula capability from a relay NOTICE or HULA frame. */
export async function handleProtectedRelayPayload(
  payload: unknown,
): Promise<boolean> {
  const cap = parseOpenClawWorkspaceMcpCapability(payload);
  if (!cap) return false;
  await applyOpenClawWorkspaceGrant({
    url: cap.mcp.url,
    authorization: cap.mcp.authorization,
    expiresAt: cap.mcp.expiresAt,
    relay: cap.relay,
    connectedViaRelay: true,
  });
  return true;
}
