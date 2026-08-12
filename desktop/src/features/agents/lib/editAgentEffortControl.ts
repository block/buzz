import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { EnvVarsValue } from "../ui/EnvVarsEditor";
import {
  deriveAgentConfigFieldModel,
  getRenderableEffortField,
  type RuntimeCatalogStatus,
} from "./agentConfigCore";

/** Descriptor-backed presentation data for the per-agent edit effort control. */
export function deriveEditAgentEffortControl({
  catalogStatus,
  envVars,
  model,
  provider,
  runtime,
  runtimeId,
}: {
  catalogStatus: RuntimeCatalogStatus;
  envVars: EnvVarsValue;
  model?: string;
  provider?: string;
  runtime?: AcpRuntimeCatalogEntry;
  runtimeId: string;
}): { persistenceKey: string; provider: string } | undefined {
  if (catalogStatus !== "ready") return undefined;

  const effortField = getRenderableEffortField(
    deriveAgentConfigFieldModel({
      config: {
        env_vars: envVars,
        model: model?.trim() || null,
        preferred_runtime: runtimeId || null,
        provider: provider?.trim() || null,
      },
      runtime,
      scope: "instance",
    }),
  );
  if (effortField?.currentPersistence.kind !== "envVar") return undefined;

  return {
    persistenceKey: effortField.currentPersistence.key,
    // Native effort currently belongs to Claude's implicit Anthropic provider;
    // unlike Buzz Agent, it has no provider picker to supply this value.
    provider:
      provider?.trim() ||
      (effortField.optionSource === "harnessNative" ? "anthropic" : ""),
  };
}
