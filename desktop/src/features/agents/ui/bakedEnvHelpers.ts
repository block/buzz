/**
 * Pure helper functions for displaying baked build env values in the global
 * agent config card. Extracted into their own module so unit tests can import
 * them without pulling in React, Tauri IPC, or TanStack Query.
 */

import {
  BUZZ_AGENT_THINKING_EFFORT,
  normalizeEffortValue,
} from "./buzzAgentConfig";

const BAKED_STRUCTURED_KEYS_BASE = new Set([
  "BUZZ_AGENT_PROVIDER",
  "BUZZ_AGENT_MODEL",
]);

/**
 * Returns the set of baked env keys hidden by structured controls.
 * Only the current runtime's native effort key is hidden; a baked legacy key
 * for a non-buzz-agent runtime remains visible as an advanced row (Delta-4).
 */
export function bakedStructuredKeys(
  nativeEffortKey: string | undefined,
): Set<string> {
  const keys = new Set(BAKED_STRUCTURED_KEYS_BASE);
  if (nativeEffortKey) keys.add(nativeEffortKey);
  return keys;
}

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

/**
 * Resolve the baked build effort for a runtime's native key, normalized through
 * the full effort contract: alias→canonical, invalid→null.
 *
 * This is the shared resolver for every baked-effort display surface
 * (`AgentConfigFields` global/onboarding and `getInheritedAgentDefaults`).
 * Both consumers must route through here so alias resolution and invalid-value
 * suppression are applied consistently at display time.
 *
 * Returns `null` when:
 * - the key is absent from baked env;
 * - the entry is masked;
 * - the raw value is empty;
 * - the value is invalid for the runtime's static vocabulary.
 */
export function resolveBakedEffort(
  bakedEnv: readonly BakedEnvEntry[] | undefined,
  nativeKey: string,
  runtime:
    | {
        acceptedEffortValues?: readonly string[] | null;
        effortAliases?: ReadonlyArray<readonly [string, string]> | null;
      }
    | null
    | undefined,
): string | null {
  const entry = bakedEnv?.find((e) => e.key === nativeKey && !e.masked);
  const raw = entry?.value.trim() ?? "";
  if (!raw) return null;
  return (
    normalizeEffortValue(
      raw,
      runtime?.acceptedEffortValues ?? null,
      runtime?.effortAliases ?? null,
    ) ?? null
  );
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
  options?: {
    /**
     * The native effort env key for the current runtime (e.g. `GOOSE_THINKING_EFFORT`).
     * When provided (and different from the legacy key), the native key is checked
     * first and baked lookup uses native-only (no legacy fallback for baked tier).
     * Defaults to `BUZZ_AGENT_THINKING_EFFORT` (buzz-agent behavior unchanged).
     */
    nativeEffortKey?: string;
    /**
     * Canonical effort values for the current runtime (from `runtime.acceptedEffortValues`).
     * Used to normalize the global native effort value before display.
     * Pass `null` for buzz-agent (per-model catalog, no static vocabulary).
     */
    acceptedEffortValues?: readonly string[] | null;
    /**
     * Effort alias pairs from `runtime.effortAliases` — the single normalization authority.
     * Passed to `normalizeEffortValue` so the TS alias table derives from Rust metadata.
     * Absent means no aliases; the fallback table has been removed.
     */
    effortAliases?: ReadonlyArray<readonly [string, string]> | null;
  },
): {
  effort: InheritedDefault;
  model: InheritedDefault;
  provider: InheritedDefault;
} {
  const nativeEffortKey =
    options?.nativeEffortKey ?? BUZZ_AGENT_THINKING_EFFORT;

  // Effort value from global config: native key only.
  // Global legacy (BUZZ_AGENT_THINKING_EFFORT for non-buzz-agent runtimes) is
  // excluded — mirrors effort_tier_alias(global_tier=true) in Rust: spawn and
  // the reader both exclude legacy at the global tier. For Goose, the legacy
  // key is buzz-agent's native key (Delta 5); consuming it here would display
  // buzz-agent's effort setting as Goose effort. Normalize the raw value so
  // global GOOSE_THINKING_EFFORT=xhigh displays as canonical "max", not "xhigh"
  // (acceptedEffortValues drives normalization; null = buzz-agent pass-through).
  const globalNativeRaw = globalConfig.env_vars[nativeEffortKey]?.trim();
  const globalNativeEffort = globalNativeRaw
    ? (normalizeEffortValue(
        globalNativeRaw,
        options?.acceptedEffortValues ?? null,
        options?.effortAliases,
      ) ?? null)
    : null;
  const globalEffortValue = globalNativeEffort || null;

  // Baked effort lookup: always use the native key (never the legacy key at
  // baked tier, mirroring the global-tier native-only rule).
  const bakedEffortKey = nativeEffortKey;

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
    effort: (() => {
      // Global config effort wins (native then legacy fallback).
      if (globalEffortValue) {
        return {
          source: "global" as InheritedDefaultSource,
          value: globalEffortValue,
        };
      }
      // Baked effort — native-only lookup for non-buzz-agent runtimes.
      // Normalize through the shared resolver (alias→canonical, invalid→absent).
      const bakedValue = resolveBakedEffort(bakedEnv, bakedEffortKey, options);
      return bakedValue
        ? { source: "build" as InheritedDefaultSource, value: bakedValue }
        : { source: null, value: "" };
    })(),
  };
}
