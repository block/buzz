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
  /** Existing persisted provider selections remain valid while their probe loads. */
  allowUnprobedProvider?: boolean;
};

export const emptyWhereToRunDraft: WhereToRunDraft = {
  runOn: "local",
  providerConfig: {},
  probedProvider: null,
};

export function providerConfigComplete(draft: WhereToRunDraft): boolean {
  if (draft.runOn === "local") return true;
  if (!draft.probedProvider) return draft.allowUnprobedProvider === true;
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

export function backendIntentToManagedAgentBackend(
  backendIntent: BackendIntent | null,
): ManagedAgentBackend {
  return backendIntent
    ? {
        type: "provider" as const,
        id: backendIntent.id,
        config: backendIntent.config,
      }
    : { type: "local" as const };
}
