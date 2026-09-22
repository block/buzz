import type { ObserverEvent, PendingPermissionResolution } from "./agentSessionTypes";
import { asRecord, asString } from "./agentSessionUtils";

export function describePermissionRequest(payload: Record<string, unknown>) {
  const params = asRecord(payload.params);
  const toolCall = asRecord(params.toolCall);
  const rawInput = asRecord(toolCall.rawInput);
  const title =
    asString(params.title) ??
    asString(params.message) ??
    asString(params.reason) ??
    asString(toolCall.title) ??
    "Permission requested";
  const toolCallId =
    asString(params.toolCallId) ??
    asString(params.tool_call_id) ??
    asString(toolCall.toolCallId) ??
    asString(toolCall.id);
  const command = asString(rawInput.command);
  const cwd = asString(rawInput.cwd);
  const toolText = Array.isArray(toolCall.content)
    ? toolCall.content
        .map((content) => asString(asRecord(content).text))
        .filter((text): text is string => Boolean(text))
        .join("\n")
    : null;
  const options = Array.isArray(params.options)
    ? params.options
        .map((option) => {
          const record = asRecord(option);
          return (
            asString(record.name) ??
            asString(record.kind) ??
            asString(record.optionId)
          );
        })
        .filter((option): option is string => Boolean(option))
    : [];
  const detail: string[] = [];
  if (title !== "Permission requested") detail.push(title);
  if (toolCallId) detail.push(`Tool call: ${toolCallId}`);
  if (command) detail.push(`Command: ${command}`);
  if (cwd) detail.push(`Working directory: ${cwd}`);
  if (toolText) detail.push(toolText);
  if (options.length > 0) detail.push(`Options: ${options.join(", ")}`);

  const optionNames = new Map<string, string>();
  if (Array.isArray(params.options)) {
    for (const option of params.options) {
      const record = asRecord(option);
      const optionId = asString(record.optionId);
      const kind = asString(record.kind);
      if (optionId && kind) {
        optionNames.set(optionId, kind);
      }
    }
  }

  return {
    title,
    text: detail.join("\n"),
    optionNames,
    descriptor: {
      renderClass: "permission" as const,
      label: "Permission requested",
      preview: title,
      action: { verb: "Requested", object: title },
      tone: "admin" as const,
      operation: "session/request_permission",
      object: title,
      source: "acp" as const,
      groupKey: "permission:request",
    },
  };
}

/**
 * Format a human-readable outcome label from a permission response.
 * kind values from ACP: allow_once, allow_always, reject_once, reject_always.
 * "reject_*" kinds are denials; anything else that is selected is an approval.
 */
export function describePermissionOutcome(
  outcome: string,
  optionId: string | null,
  optionNames: Map<string, string>,
): string {
  if (outcome === "cancelled") {
    return "Cancelled";
  }
  if (outcome === "selected" && optionId) {
    const kind = optionNames.get(optionId) ?? optionId;
    const isDenial = kind.startsWith("reject");
    const verb = isDenial ? "Denied" : "Approved";
    return `${verb} (${kind})`;
  }
  return outcome;
}

/**
 * Stable map key for a JSON-RPC id, which may be a string or a finite number
 * per the spec. Using JSON.stringify avoids collisions between the number 1 and
 * the string "1". Returns null for null, undefined, or non-id values (objects,
 * booleans) so callers can gate on presence without a separate type check.
 */
export function jsonRpcId(value: unknown): string | null {
  const rawId = rawJsonRpcId(value);
  return rawId === null ? null : JSON.stringify(rawId);
}

/** Preserve the wire value for a later authenticated owner-resolution call. */
export function rawJsonRpcId(value: unknown): string | number | null {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  return null;
}

/**
 * Project durable records into the transcript only after the matching runtime
 * has an open observer connection. A dead harness cannot resume the ACP read,
 * so its persisted Pending row remains visible as an unavailable history row
 * without Desktop mutating the single-writer ledger.
 */
export function projectPermissionLedgerEvents(
  records: unknown[],
  runtimeCanResolve: boolean,
): ObserverEvent[] {
  return records.flatMap((record, index) => {
    if (!record || typeof record !== "object") return [];
    const value = record as Record<string, unknown>;
    const channelId =
      typeof value.channelId === "string" ? value.channelId : null;
    const sessionId =
      typeof value.sessionId === "string" ? value.sessionId : null;
    const turnId = typeof value.turnId === "string" ? value.turnId : null;
    const timestamp =
      typeof value.updatedAt === "string"
        ? value.updatedAt
        : new Date(0).toISOString();
    const state = asRecord(value.state);
    const payload =
      runtimeCanResolve || asString(state.kind) !== "pending"
        ? value
        : { ...value, state: { ...state, kind: "abandoned" } };
    return [
      {
        seq: -1 - index,
        timestamp,
        kind: "permission_ledger",
        agentIndex: null,
        channelId,
        sessionId,
        turnId,
        payload,
      },
    ];
  });
}

/**
 * ACP permits request IDs to repeat in different sessions. A channel can also
 * hold multiple thread sessions, so no one of those values is a safe owner
 * decision correlator on its own. Preserve the JSON type in `requestId` so
 * numeric 1 and string "1" remain distinct.
 */
export function permissionIdentity(
  channelId: string | null,
  sessionId: string | null,
  turnId: string | null,
  requestId: string,
) {
  return `${channelId ?? "global"}:${sessionId ?? "unknown-session"}:${turnId ?? "unknown-turn"}:${requestId}`;
}

export function ownerResolutionFromPending(
  payload: Record<string, unknown>,
  event: ObserverEvent,
): PendingPermissionResolution | null {
  const requestId = payload.requestId;
  const actionDigest = asString(payload.actionDigest);
  const sessionId = asString(payload.sessionId);
  const turnId = event.turnId;
  if (
    (typeof requestId !== "string" && typeof requestId !== "number") ||
    !actionDigest ||
    !sessionId ||
    !turnId ||
    !Array.isArray(payload.options)
  ) {
    return null;
  }
  const options = payload.options.flatMap((value) => {
    const option = asRecord(value);
    const optionId = asString(option.optionId);
    if (!optionId) return [];
    return [
      {
        optionId,
        label: asString(option.name) ?? asString(option.kind) ?? optionId,
      },
    ];
  });
  return options.length > 0
    ? { turnId, sessionId, requestId, actionDigest, options }
    : null;
}
