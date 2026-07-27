import type {
  AgentActivityDescriptor,
  AgentPermissionOption,
} from "./agentSessionTypes";
import { asRecord, asString } from "./agentSessionUtils";

export type PermissionRequestDescription = {
  title: string;
  text: string;
  optionNames: Map<string, string>;
  permissionOptions: AgentPermissionOption[];
  descriptor: AgentActivityDescriptor;
};

/**
 * Interactive managed-runtime consent is intentionally one-shot. Persistent
 * runtime choices stay visible in the transcript for auditability, but they
 * are never returned as actionable controls.
 */
export function oneShotPermissionOptions(
  options: AgentPermissionOption[],
): AgentPermissionOption[] {
  return options.filter(
    (option) => option.kind === "allow_once" || option.kind === "reject_once",
  );
}

export function describePermissionRequest(
  payload: Record<string, unknown>,
): PermissionRequestDescription {
  const params = asRecord(payload.params);
  const toolCall = asRecord(params.toolCall);
  const title =
    asString(toolCall.title) ??
    asString(params.title) ??
    asString(params.message) ??
    asString(params.reason) ??
    "Permission requested";
  const toolCallId =
    asString(toolCall.toolCallId) ??
    asString(toolCall.tool_call_id) ??
    asString(params.toolCallId) ??
    asString(params.tool_call_id);
  const permissionOptions: AgentPermissionOption[] = Array.isArray(
    params.options,
  )
    ? params.options
        .map((option) => {
          const record = asRecord(option);
          const optionId = asString(record.optionId);
          const kind = asString(record.kind);
          if (!optionId || !kind) return null;
          return {
            optionId,
            kind,
            name: asString(record.name) ?? kind,
          };
        })
        .filter((option): option is AgentPermissionOption => option !== null)
    : [];
  const detail: string[] = [];
  if (title !== "Permission requested") detail.push(title);
  if (toolCallId) detail.push(`Tool call: ${toolCallId}`);
  if (permissionOptions.length > 0) {
    detail.push(
      `Options: ${permissionOptions.map((option) => option.name).join(", ")}`,
    );
  }

  const optionNames = new Map<string, string>();
  for (const option of permissionOptions) {
    optionNames.set(option.optionId, option.kind);
  }

  return {
    title,
    text: detail.join("\n"),
    optionNames,
    permissionOptions,
    descriptor: {
      renderClass: "permission",
      label: "Permission requested",
      preview: title,
      action: { verb: "Requested", object: title },
      tone: "admin",
      operation: "session/request_permission",
      object: title,
      source: "acp",
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
