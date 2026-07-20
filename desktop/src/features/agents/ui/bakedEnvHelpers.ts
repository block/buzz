/**
 * Pure helper functions for displaying baked build env values in the global
 * agent config card. Extracted into their own module so unit tests can import
 * them without pulling in React, Tauri IPC, or TanStack Query.
 */

/**
 * Return the provider option label for the zero-value (inherit) option when a
 * baked provider is present. Falls back to the raw provider id when the id
 * doesn't appear in the options table.
 *
 * Used in AgentDefaultsSettingsCard to relabel the provider dropdown's
 * empty-selection option when a baked build provider is set.
 */
export function getBakedProviderInheritLabel(
  bakedProviderId: string,
  options: readonly { id: string; label: string }[],
): string {
  const match = options.find((o) => o.id === bakedProviderId);
  const friendlyName = match ? match.label : bakedProviderId;
  return `${friendlyName} (inherited from build)`;
}

export type InheritedDefaultSource = "build" | "global" | null;

export type InheritedDefault = {
  source: InheritedDefaultSource;
  value: string;
};

type BakedEnvEntry = {
  key: string;
  masked: boolean;
  value: string;
};

/**
 * Resolve a dialog's inherited default with the backend's precedence:
 * persisted global config wins, and a safe baked build value fills an unset
 * global field. Masked build values are never eligible for UI labels.
 */
export function resolveInheritedDefault(
  globalValue: string | null | undefined,
  bakedEnv: readonly BakedEnvEntry[] | undefined,
  bakedKey: string,
): InheritedDefault {
  const global = globalValue?.trim() ?? "";
  if (global) return { source: "global", value: global };

  const baked = bakedEnv?.find(
    (entry) => entry.key === bakedKey && !entry.masked,
  );
  const value = baked?.value.trim() ?? "";
  return value ? { source: "build", value } : { source: null, value: "" };
}

export function getBakedModelInheritLabel(bakedModelId: string): string {
  return `Inherit build default (${bakedModelId})`;
}

function providerModelEnvKey(provider: string): string | null {
  switch (provider.trim().toLowerCase()) {
    case "databricks":
    case "databricks_v2":
    case "databricks-v2":
      return "DATABRICKS_MODEL";
    case "anthropic":
      return "ANTHROPIC_MODEL";
    case "openai":
    case "openai-compat":
      return "OPENAI_COMPAT_MODEL";
    case "gemini":
      return "GEMINI_MODEL";
    default:
      return null;
  }
}

/** Resolve the concrete model beneath the structured global model override. */
export function getGlobalModelFallback(
  bakedEnv: readonly BakedEnvEntry[] | undefined,
  provider: string,
  globalEnv: Readonly<Record<string, string>> = {},
): string | null {
  const universal = bakedEnv?.find(
    (entry) => entry.key === "BUZZ_AGENT_MODEL" && !entry.masked,
  )?.value;
  if (universal?.trim()) return universal.trim();

  const providerKey = providerModelEnvKey(provider);
  if (!providerKey) return provider === "relay-mesh" ? "auto" : null;
  const globalProviderModel = globalEnv[providerKey]?.trim();
  if (globalProviderModel) return globalProviderModel;
  const providerModel = bakedEnv?.find(
    (entry) => entry.key === providerKey && !entry.masked,
  )?.value;
  return providerModel?.trim() || null;
}

/**
 * Global env keys are arbitrary user input. Count only ordinary configuration
 * names in the collapsed summary so it never calls attention to credentials.
 */
export function countNonSecretInheritedEnvVars(
  envVars: Record<string, string>,
  excludedKey?: string | null,
): number {
  return Object.entries(envVars).filter(
    ([key, value]) =>
      key !== excludedKey &&
      value.length > 0 &&
      !/(?:_KEY|_TOKEN|_SECRET|_PASSWORD)$/i.test(key),
  ).length;
}

/** Build a source-aware, value-safe summary for collapsed Advanced fields. */
export function getAdvancedInheritedSummary(
  effort: InheritedDefault,
  globalEnvCount: number,
): string {
  if (!effort.value && globalEnvCount === 0) return "";

  const globalEnvLabel =
    globalEnvCount > 0
      ? `${globalEnvCount} global env var${globalEnvCount > 1 ? "s" : ""}`
      : "";
  if (effort.source === "build" && globalEnvCount === 0) {
    return `Using build defaults: effort ${effort.value}`;
  }
  if (effort.source === "build") {
    return `Using inherited defaults: build effort ${effort.value} · ${globalEnvLabel}`;
  }

  const parts = [
    ...(effort.value ? [`effort ${effort.value}`] : []),
    ...(globalEnvLabel ? [globalEnvLabel] : []),
  ];
  return `Using agent defaults: ${parts.join(" · ")}`;
}

export function getInheritedAgentDefaults(
  globalConfig: {
    env_vars: Record<string, string>;
    model: string | null;
    provider: string | null;
  },
  bakedEnv: readonly BakedEnvEntry[] | undefined,
): {
  effort: InheritedDefault;
  model: InheritedDefault;
  provider: InheritedDefault;
} {
  return {
    provider: resolveInheritedDefault(
      globalConfig.provider,
      bakedEnv,
      "BUZZ_AGENT_PROVIDER",
    ),
    model: (() => {
      const structured = resolveInheritedDefault(
        globalConfig.model,
        bakedEnv,
        "BUZZ_AGENT_MODEL",
      );
      if (structured.value) return structured;
      const provider = resolveInheritedDefault(
        globalConfig.provider,
        bakedEnv,
        "BUZZ_AGENT_PROVIDER",
      );
      const fallback = getGlobalModelFallback(
        bakedEnv,
        provider.value,
        globalConfig.env_vars,
      );
      return fallback
        ? { source: "build", value: fallback }
        : { source: null, value: "" };
    })(),
    effort: resolveInheritedDefault(
      globalConfig.env_vars.BUZZ_AGENT_THINKING_EFFORT,
      bakedEnv,
      "BUZZ_AGENT_THINKING_EFFORT",
    ),
  };
}
