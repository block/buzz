/**
 * Promote certain machine-readable `lastError` strings to user-facing copy.
 *
 * The error classification seam flows like this:
 *   buzz-agent — classifies LLM failures into `AgentError` variants with
 *                  JSON-RPC codes (`-32001` auth, `-32002` model-not-found,
 *                  `-32000` generic), defined in `crates/buzz-agent/src/types.rs`.
 *   buzz-acp   — preserves the code structurally in
 *                  `AcpError::AgentError { code, message }`, whose Display is
 *                  `"Agent reported error (code N): message"`, and includes
 *                  `code` in `turn_error` observer events.
 *   desktop supervisor — on nonzero exit, recovers `{ message, code }` from
 *                  the log tail (`managed_agents/storage.rs`) into
 *                  `ManagedAgent.lastError` / `lastErrorCode`.
 *
 * This function dispatches on the numeric code first (works for any harness),
 * then recovers a code embedded in the message string (handles records where
 * the `lastErrorCode` field was lost, e.g. downgrade or pre-code records with
 * new-format strings), then falls back to legacy string prefixes for records
 * written before structured codes existed.
 *
 * Returns:
 *  - null when there's nothing to show (null/empty lastError).
 *  - A `{ severity: "denied"; copy: string }` object for the auth-failure
 *    and model-not-found cases, so the UI can render with the right visual
 *    weight (destructive).
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

const EMBEDDED_CODE_RE = /^Agent reported error \(code (-?\d+)\): /;

function recoverEmbeddedCode(trimmed: string): number | null {
  const match = EMBEDDED_CODE_RE.exec(trimmed);
  return match ? Number(match[1]) : null;
}

export function friendlyAgentLastError(
  raw: string | null,
  code?: number | null,
): FriendlyAgentLastError | null {
  if (raw == null) return null;
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;

  // Structured code first; a code embedded in the message string is the
  // same signal recovered from a record that lost the field.
  const effectiveCode = Number.isFinite(code)
    ? (code as number)
    : recoverEmbeddedCode(trimmed);
  if (effectiveCode != null) {
    switch (effectiveCode) {
      case -32001:
        return { severity: "denied", copy: RELAY_MESH_DENIED_COPY };
      case -32002:
        return { severity: "denied", copy: MODEL_NOT_FOUND_COPY };
    }
    // A structured code we don't recognize is authoritative — don't let
    // string patterns cross-classify it.
    return { severity: "generic", copy: trimmed };
  }

  // Legacy string fallback for records written before codes existed.
  // Match either the unwrapped buzz-agent prefix or the buzz-acp v0 wrap.
  if (
    trimmed.startsWith("Agent reported error: llm auth:") ||
    trimmed.startsWith("llm auth:")
  ) {
    return { severity: "denied", copy: RELAY_MESH_DENIED_COPY };
  }

  return { severity: "generic", copy: trimmed };
}

/**
 * Convenience for `turn_error` / `agent_panic` observer payloads: coerce the
 * payload's untyped `code` JSON value and return the display copy, falling
 * back to the raw error text when no classification applies.
 */
export function friendlyTurnErrorCopy(raw: string, code: unknown): string {
  const numeric = code == null ? null : Number(code);
  const safe = Number.isFinite(numeric) ? (numeric as number) : null;
  return friendlyAgentLastError(raw, safe)?.copy ?? raw;
}
