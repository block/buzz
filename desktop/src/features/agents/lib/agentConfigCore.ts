import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { BUZZ_AGENT_THINKING_EFFORT } from "../ui/buzzAgentConfig";

export type AgentConfigScope =
  | "onboarding"
  | "global"
  | "definition"
  | "instance";

export type DependentValuePolicy = {
  onContextChange: "resetDependentValues";
  onCatalogMismatch: "explainOnly" | "onboardingCleanup";
};

type NormalizedFieldPersistence = {
  kind: "normalizedField";
  field: "provider" | "model";
};

type EnvVarPersistence = {
  kind: "envVar";
  key: string;
};

type AcpConfigOptionPersistence = {
  kind: "acpConfigOption";
  id: string;
  category: string;
};

type UnavailablePersistence = {
  kind: "unavailable";
};

export type AgentConfigFieldDescriptor =
  | {
      kind: "provider";
      optionSource: "providerCatalog";
      persistence: NormalizedFieldPersistence;
      targetApplication: { kind: "envVar"; key: string };
      render: "control";
      value: string | null;
    }
  | {
      kind: "model";
      optionSource: "acpModels";
      persistence: NormalizedFieldPersistence;
      targetApplication:
        | { kind: "envVar"; key: string }
        | { kind: "acpNative" };
      render: "control";
      value: string | null;
    }
  | {
      kind: "effort";
      optionSource:
        | "buzzAgentCatalog"
        | "legacyProviderModelCatalog"
        | "harnessNative";
      currentPersistence:
        | EnvVarPersistence
        | AcpConfigOptionPersistence
        | UnavailablePersistence;
      targetApplication:
        | { kind: "envVar"; key: string }
        | { kind: "acpConfigOption"; id: string; category: string };
      render: "control" | "deferredUntilNativeOptionsAvailable";
      value: string | null;
    };

export type AgentConfigOmission = {
  kind: "effort";
  reason: "ownedByModelId" | "unsupportedByHarness";
};

export type AgentConfigFieldModel = {
  fields: AgentConfigFieldDescriptor[];
  omissions: AgentConfigOmission[];
  dependentValuePolicy: DependentValuePolicy;
};

function valueFromEnv(config: GlobalAgentConfig, key: string) {
  return config.env_vars[key]?.trim() || null;
}

/**
 * Derives the harness-scoped field model consumed by agent config renderers.
 *
 * The runtime catalog is authoritative for environment-variable application.
 * Harness-native ACP options are named here until discovery exposes them to the
 * desktop; descriptors marked deferred must not be rendered as generic fields.
 */
export function deriveAgentConfigFieldModel({
  config,
  runtime,
  scope,
}: {
  config: GlobalAgentConfig;
  runtime: AcpRuntimeCatalogEntry | undefined;
  scope: AgentConfigScope;
}): AgentConfigFieldModel {
  const fields: AgentConfigFieldDescriptor[] = [];
  const omissions: AgentConfigOmission[] = [];

  if (runtime?.providerEnvVar) {
    fields.push({
      kind: "provider",
      optionSource: "providerCatalog",
      persistence: { kind: "normalizedField", field: "provider" },
      targetApplication: { kind: "envVar", key: runtime.providerEnvVar },
      render: "control",
      value: config.provider,
    });
  }

  fields.push({
    kind: "model",
    optionSource: "acpModels",
    persistence: { kind: "normalizedField", field: "model" },
    targetApplication: runtime?.modelEnvVar
      ? { kind: "envVar", key: runtime.modelEnvVar }
      : { kind: "acpNative" },
    render: "control",
    value: config.model,
  });

  if (runtime?.thinkingEnvVar) {
    fields.push({
      kind: "effort",
      optionSource:
        runtime.id === "buzz-agent"
          ? "buzzAgentCatalog"
          : "legacyProviderModelCatalog",
      currentPersistence: {
        kind: "envVar",
        key: BUZZ_AGENT_THINKING_EFFORT,
      },
      targetApplication: { kind: "envVar", key: runtime.thinkingEnvVar },
      render: "control",
      value: valueFromEnv(config, BUZZ_AGENT_THINKING_EFFORT),
    });
  } else if (runtime?.id === "claude") {
    fields.push({
      kind: "effort",
      optionSource: "harnessNative",
      currentPersistence: { kind: "unavailable" },
      targetApplication: {
        kind: "acpConfigOption",
        id: "effort",
        category: "thought_level",
      },
      render: "deferredUntilNativeOptionsAvailable",
      value: null,
    });
  } else {
    omissions.push({
      kind: "effort",
      reason:
        runtime?.id === "codex" ? "ownedByModelId" : "unsupportedByHarness",
    });
  }

  return {
    fields,
    omissions,
    dependentValuePolicy: {
      onContextChange: "resetDependentValues",
      onCatalogMismatch:
        scope === "onboarding" ? "onboardingCleanup" : "explainOnly",
    },
  };
}

export function hasRenderableAgentConfigField(
  model: AgentConfigFieldModel,
  kind: AgentConfigFieldDescriptor["kind"],
) {
  return model.fields.some(
    (field) => field.kind === kind && field.render === "control",
  );
}

export function getRenderableEffortField(
  model: AgentConfigFieldModel,
): Extract<AgentConfigFieldDescriptor, { kind: "effort" }> | undefined {
  return model.fields.find(
    (field): field is Extract<AgentConfigFieldDescriptor, { kind: "effort" }> =>
      field.kind === "effort" && field.render === "control",
  );
}
