/**
 * Promote certain machine-readable `lastError` strings to user-facing copy.
 *
 * The mesh-llm seam (Max's commit `5196203…`) flows like this:
 *   buzz-agent — gets HTTP 401/403 from the OpenAI-compatible mesh endpoint
 *                  → raises `AgentError::LlmAuth("…")` (json_rpc_code `-32001`,
 *                    Display prefix `"llm auth: …"`)
 *   buzz-acp — wraps it as `AcpError::AgentError("Agent reported error: llm auth: …")`
 *   desktop managed-agent supervisor — on nonzero exit, scans `read_log_tail`
 *                  for `"Agent reported error:"` / `"llm auth:"` and persists
 *                  that line into `ManagedAgent.lastError` instead of the
 *                  generic `"harness exited with status …"`.
 *
 * v1 caveat (named, not hidden): the typed `-32001` code never reaches
 * desktop structurally — desktop only supervises the child process and ACP's
 * `ObserverHandle` is in-process inside that child. So `lastError` is the
 * recovered string, not the original code. The follow-up to make this 9/10
 * structural is an ACP status file or desktop-owned observer sink; that lives
 * elsewhere. For now we match the string this function exists to render.
 *
 * Returns:
 *  - null when there's nothing to show (null/empty lastError).
 *  - A `{ severity: "denied"; copy: string }` object for the auth-failure
 *    case, so the UI can render with the right visual weight (destructive).
 *  - A `{ severity: "generic"; copy: string }` pass-through for any other
 *    lastError, so generic harness exits still surface their text instead of
 *    being swallowed.
 */
export type FriendlyAgentLastError =
  | { severity: "denied"; copy: string }
  | { severity: "generic"; copy: string };

/**
 * The exact copy for the relay-mesh denial. Centralized as a constant so the
 * test asserts the user-facing string verbatim rather than a fuzzy pattern.
 */
export const RELAY_MESH_DENIED_COPY =
  "Relay mesh denied this agent — check your relay membership.";

export const MODEL_NOT_FOUND_COPY =
  "The configured model is not available — open agent settings and select a different one from the dropdown.";

export function friendlyAgentLastError(
  raw: string | null,
  code?: number | null,
): FriendlyAgentLastError | null {
  if (raw == null) return null;
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;

  // Structured code path — no string matching, works across all providers.
  if (code != null) {
    switch (code) {
      case -32001:
        return { severity: "denied", copy: RELAY_MESH_DENIED_COPY };
      case -32002:
        return { severity: "denied", copy: MODEL_NOT_FOUND_COPY };
    }
  }

  // Match either the unwrapped buzz-agent prefix or the buzz-acp wrap.
  // The desktop supervisor recovers whichever appears first in the log tail.
  if (
    trimmed.startsWith("Agent reported error: llm auth:") ||
    trimmed.startsWith("llm auth:")
  ) {
    return { severity: "denied", copy: RELAY_MESH_DENIED_COPY };
  }

  return { severity: "generic", copy: trimmed };
}
