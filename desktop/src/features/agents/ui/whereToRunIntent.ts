import type { BackendIntent } from "../lib/instanceInputForDefinition";
import type {
  BackendProviderProbeResult,
  ManagedAgentBackend,
} from "@/shared/api/types";
import { coerceConfigValues } from "./ProviderConfigFields";

/** Draft state of the optional remote-backend selector. */
export type WhereToRunDraft = {
  runOn: "local" | string;
  providerConfig: Record<string, string>;
  probedProvider: BackendProviderProbeResult | null;
};

export const emptyWhereToRunDraft: WhereToRunDraft = {
  runOn: "local",
  providerConfig: {},
  probedProvider: null,
};

/**
 * Fold a completed probe into the draft the user has *now* — not the draft
 * that existed when the probe started. Schema defaults prefill only the keys
 * the user has not touched: anything already in `providerConfig` (typed while
 * the probe was in flight) wins over the default. Overwriting instead of
 * merging is the "Typewriter Eraser" bug — every probe resolution silently
 * erased in-flight keystrokes.
 */
export function applyProbeResult(
  current: WhereToRunDraft,
  result: BackendProviderProbeResult,
): WhereToRunDraft {
  const defaults: Record<string, string> = {};
  const properties =
    (result.config_schema as Record<string, unknown> | undefined)?.properties ??
    {};
  for (const [key, property] of Object.entries(properties) as [
    string,
    Record<string, unknown>,
  ][]) {
    if (property.default != null) defaults[key] = String(property.default);
  }
  return {
    ...current,
    probedProvider: result,
    providerConfig: { ...defaults, ...current.providerConfig },
  };
}

/** Prefix marking a `runOn` value that targets a paired execution node. */
export const EXECUTION_NODE_RUN_ON_PREFIX = "execution-node:";

/** Encode an execution-node id as a `runOn` selector value. */
export function executionNodeRunOnValue(nodeId: string): string {
  return `${EXECUTION_NODE_RUN_ON_PREFIX}${nodeId}`;
}

export function isExecutionNodeRunOn(runOn: string): boolean {
  return runOn.startsWith(EXECUTION_NODE_RUN_ON_PREFIX);
}

/** Extract the node id from an execution-node `runOn` value, or null. */
export function parseExecutionNodeRunOn(runOn: string): string | null {
  return isExecutionNodeRunOn(runOn)
    ? runOn.slice(EXECUTION_NODE_RUN_ON_PREFIX.length)
    : null;
}

/**
 * Draft pre-selected to an existing agent's persisted backend, for the edit
 * dialog. Provider config is deliberately NOT reconstructed from the stored
 * backend: an unchanged selection resolves to no transition at all (see
 * `resolveBackendChangeIntent`), and a re-picked provider re-probes fresh.
 */
export function whereToRunDraftForBackend(
  backend: ManagedAgentBackend,
): WhereToRunDraft {
  if (backend.type === "execution_node") {
    return {
      ...emptyWhereToRunDraft,
      runOn: executionNodeRunOnValue(backend.nodeId),
    };
  }
  if (backend.type === "provider") {
    return { ...emptyWhereToRunDraft, runOn: backend.id };
  }
  return emptyWhereToRunDraft;
}

/**
 * A backend the edit dialog can move an agent to. Unlike the create flow —
 * where "local" is the absence of a `BackendIntent` — a swap back to local is
 * an explicit transition, so it gets its own variant.
 */
export type BackendChangeIntent = { type: "local" } | BackendIntent;

/**
 * Resolve the edit dialog's draft against the agent's persisted backend.
 * `null` = the selection matches the current backend and no transition runs.
 */
export function resolveBackendChangeIntent(
  draft: WhereToRunDraft,
  currentBackend: ManagedAgentBackend,
): BackendChangeIntent | null {
  const intent = resolveBackendIntent(draft);
  if (intent === null) {
    return currentBackend.type === "local" ? null : { type: "local" };
  }
  if (intent.type === "execution-node") {
    return currentBackend.type === "execution_node" &&
      currentBackend.nodeId === intent.nodeId
      ? null
      : intent;
  }
  return currentBackend.type === "provider" && currentBackend.id === intent.id
    ? null
    : intent;
}

/**
 * Edit-dialog effective change: an unchanged selection normally means no
 * transition — EXCEPT an execution-node agent with no confirmed workload
 * (a create- or swap-deploy that failed after the backend was persisted).
 * Re-saving converges that half-deployed state by re-running the
 * authoritative deploy; the backend transition itself is then a no-op.
 */
export function resolveEffectiveBackendChange(
  draft: WhereToRunDraft,
  agent: { backend: ManagedAgentBackend; backendAgentId: string | null },
): BackendChangeIntent | null {
  const intent = resolveBackendChangeIntent(draft, agent.backend);
  if (intent) return intent;
  if (agent.backend.type === "execution_node" && !agent.backendAgentId) {
    return { type: "execution-node", nodeId: agent.backend.nodeId };
  }
  return null;
}

export function providerConfigComplete(draft: WhereToRunDraft): boolean {
  if (draft.runOn === "local" || isExecutionNodeRunOn(draft.runOn)) return true;
  if (!draft.probedProvider) return false;
  const schema = draft.probedProvider.config_schema as
    | Record<string, unknown>
    | undefined;
  const required: string[] = (schema?.required as string[] | undefined) ?? [];
  return required.every(
    (key) => (draft.providerConfig[key] ?? "").trim().length > 0,
  );
}

export function canSubmitWhereToRun(draft: WhereToRunDraft): boolean {
  return providerConfigComplete(draft);
}

export function resolveBackendIntent(
  draft: WhereToRunDraft,
): BackendIntent | null {
  if (draft.runOn === "local") return null;
  const executionNodeId = parseExecutionNodeRunOn(draft.runOn);
  if (executionNodeId !== null) {
    return {
      type: "execution-node",
      nodeId: executionNodeId,
    };
  }
  return {
    type: "provider",
    id: draft.runOn,
    config: coerceConfigValues(
      draft.providerConfig,
      draft.probedProvider?.config_schema,
    ),
  };
}
