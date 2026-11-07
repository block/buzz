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

export function providerConfigComplete(draft: WhereToRunDraft): boolean {
  if (draft.runOn === "local") return true;
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
  return {
    type: "provider",
    id: draft.runOn,
    config: coerceConfigValues(
      draft.providerConfig,
      draft.probedProvider?.config_schema,
    ),
  };
}

export function draftFromBackend(
  backend: ManagedAgentBackend,
): WhereToRunDraft {
  if (backend.type === "local") return emptyWhereToRunDraft;
  const providerConfig: Record<string, string> = {};
  for (const [key, value] of Object.entries(backend.config)) {
    if (value != null) providerConfig[key] = String(value);
  }
  return { runOn: backend.id, providerConfig, probedProvider: null };
}

function sameBackend(a: ManagedAgentBackend, b: ManagedAgentBackend): boolean {
  if (a.type === "local" || b.type === "local") return a.type === b.type;
  const keys = Object.keys(a.config);
  return (
    a.id === b.id &&
    keys.length === Object.keys(b.config).length &&
    keys.every(
      (key) => JSON.stringify(a.config[key]) === JSON.stringify(b.config[key]),
    )
  );
}

export type BackendEdit = {
  /** Absent when the draft is closed or matches the saved backend. */
  backend?: ManagedAgentBackend;
  /** True when moving away from a provider that may still run this agent. */
  needsConfirmation: boolean;
  valid: boolean;
};

export function resolveBackendEdit(
  current: { backend: ManagedAgentBackend; backendAgentId: string | null },
  draft: WhereToRunDraft | null,
): BackendEdit {
  if (!draft) return { needsConfirmation: false, valid: true };
  const next: ManagedAgentBackend = resolveBackendIntent(draft) ?? {
    type: "local",
  };
  if (sameBackend(next, current.backend)) {
    return { needsConfirmation: false, valid: true };
  }
  const leavesProvider =
    current.backend.type === "provider" &&
    (next.type === "local" || next.id !== current.backend.id);
  return {
    backend: next,
    needsConfirmation: leavesProvider && current.backendAgentId != null,
    valid: canSubmitWhereToRun(draft),
  };
}
