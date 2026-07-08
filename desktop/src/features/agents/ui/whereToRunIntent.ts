import type { BackendIntent } from "../lib/instanceInputForDefinition";
import type { BackendProviderProbeResult } from "@/shared/api/types";
import type { MeshServeTarget } from "@/shared/api/tauriMesh";
import type { MeshAgentPresetPatch } from "@/features/mesh-compute/applyMeshAgentPreset";
import { coerceConfigValues } from "./ProviderConfigFields";

/** Draft state of the "Where to run" section, owned by the section itself. */
export type WhereToRunDraft = {
  runOn: "local" | "mesh" | string;
  providerConfig: Record<string, string>;
  probedProvider: BackendProviderProbeResult | null;
  meshModelId: string;
  meshTarget: MeshServeTarget | null;
  meshPatch: MeshAgentPresetPatch | null;
};

export const emptyWhereToRunDraft: WhereToRunDraft = {
  runOn: "local",
  providerConfig: {},
  probedProvider: null,
  meshModelId: "",
  meshTarget: null,
  meshPatch: null,
};

/**
 * Whether the provider config satisfies the probed schema's required list.
 * Mirrors the legacy dialog's `providerConfigComplete` gate: unknown schema
 * (probe pending/failed) is NOT complete for a non-local selection.
 */
export function providerConfigComplete(draft: WhereToRunDraft): boolean {
  if (draft.runOn === "local" || draft.runOn === "mesh") return true;
  if (!draft.probedProvider) return false;
  const schema = draft.probedProvider.config_schema as
    | Record<string, unknown>
    | undefined;
  const required: string[] = (schema?.required as string[] | undefined) ?? [];
  return required.every(
    (key) => (draft.providerConfig[key] ?? "").trim().length > 0,
  );
}

/**
 * Whether a create-and-start submit is allowed for this draft. Carries the
 * legacy dialog's gates: provider mode blocks until the probe succeeds and
 * required config is filled; mesh mode blocks until a concrete serve target
 * (not just a model name) is selected. Local always passes.
 *
 * When `startAfterCreate` is false there is no instance, so the draft is
 * irrelevant and submit is always allowed (the intent is discarded — see
 * resolveBackendIntent).
 */
export function canSubmitWhereToRun(
  draft: WhereToRunDraft,
  startAfterCreate: boolean,
): boolean {
  if (!startAfterCreate) return true;
  if (draft.runOn === "mesh") {
    return draft.meshModelId.trim().length > 0 && draft.meshTarget != null;
  }
  return providerConfigComplete(draft);
}

/**
 * Resolve the draft into the BackendIntent the instance mint should carry.
 *
 * Returns null for local AND whenever `startAfterCreate` is false: with the
 * start toggle off no instance exists, so a leftover provider/mesh selection
 * must be discarded at submit — never silently attached to a definition-only
 * create (the stale-intent edge).
 */
export function resolveBackendIntent(
  draft: WhereToRunDraft,
  startAfterCreate: boolean,
): BackendIntent | null {
  if (!startAfterCreate || draft.runOn === "local") {
    return null;
  }
  if (draft.runOn === "mesh") {
    if (!draft.meshTarget || !draft.meshPatch || !draft.meshModelId.trim()) {
      return null;
    }
    return {
      type: "mesh",
      modelId: draft.meshModelId.trim(),
      target: draft.meshTarget,
      patch: draft.meshPatch,
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
