import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import {
  BUZZ_AGENT_THINKING_EFFORT,
  normalizeEffortValue,
} from "../ui/buzzAgentConfig";

/**
 * Lifecycle status of the ACP runtime catalog query on a per-agent surface.
 *
 * - `loading` — query in flight; structured controls are withheld and env-var
 *   keys are not hidden (saved values remain visible as generic rows).
 * - `ready`   — query resolved; descriptors derived from `selectedRuntime`.
 * - `error`   — query failed; same gate as loading: no structured controls,
 *   keys not hidden, saved values stay visible. Distinguishable from
 *   "runtime not capable" (which is `ready` + no selectedRuntime).
 */
export type RuntimeCatalogStatus = "loading" | "ready" | "error";

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
      /** Set when a legacy key's value is being shown via fallback; callers
       * should add this key to hidden-keys to suppress the duplicate row. */
      legacyConsumedKey?: string;
    }
  | {
      kind: "maxOutputTokens" | "contextLimit" | "maxRounds";
      currentPersistence: EnvVarPersistence;
      targetApplication: { kind: "envVar"; key: string };
      render: "control";
      value: string | null;
    };

export type AgentConfigOmission = {
  kind: "effort";
  reason: "ownedByModelId" | "unsupportedByHarness";
};

/**
 * A numeric tuning descriptor: one of the three env-var-backed number fields
 * (max output tokens, context limit, max rounds).
 *
 * Defined here so both the field model derivation and the rendering surfaces
 * share a single type — avoids the type being redefined in UI layers.
 */
export type NumericDescriptor = Extract<
  AgentConfigFieldDescriptor,
  { kind: "maxOutputTokens" | "contextLimit" | "maxRounds" }
>;

export type AgentConfigFieldModel = {
  fields: AgentConfigFieldDescriptor[];
  omissions: AgentConfigOmission[];
  dependentValuePolicy: DependentValuePolicy;
};

function valueFromEnv(config: GlobalAgentConfig, key: string) {
  return config.env_vars[key]?.trim() || null;
}

/**
 * Single policy source for reading effort from a flat env-var map.
 *
 * Native key takes precedence; legacy key is the fallback only at
 * record/persona tiers (caller must pass `legacyKey` conditionally — pass
 * `null` at global/onboarding scope to enforce the tier boundary).
 *
 * Returns `{ value, legacyConsumed }` where:
 * - `value`          — the normalized canonical value, or `null` if absent/invalid
 * - `legacyConsumed` — true iff `value` came from the legacy key
 *
 * Used by both `deriveAgentConfigFieldModel` (model layer) and
 * `HarnessNativeEffortFields` (component layer) so neither duplicates the
 * native-first / valid-legacy-fallback / normalize logic.
 */
export function resolveEffortFromEnv(
  envVars: Record<string, string>,
  nativeKey: string,
  legacyKey: string | null,
  acceptedValues: readonly string[] | null,
  effortAliases?: ReadonlyArray<readonly [string, string]> | null,
): { value: string | null; legacyConsumed: boolean } {
  const nativeRaw = envVars[nativeKey]?.trim() || null;
  const legacyRaw = legacyKey ? envVars[legacyKey]?.trim() || null : null;
  const nativeValue = nativeRaw
    ? (normalizeEffortValue(nativeRaw, acceptedValues, effortAliases) ?? null)
    : null;
  const legacyValue = legacyRaw
    ? (normalizeEffortValue(legacyRaw, acceptedValues, effortAliases) ?? null)
    : null;
  const value = nativeValue ?? legacyValue;
  const legacyConsumed = !nativeValue && !!legacyValue;
  return { value, legacyConsumed };
}

/**
 * Derives the numeric descriptor set for a runtime from catalog fields.
 *
 * The returned descriptors drive `NumericTuningFields` on any surface that
 * renders numeric knobs. Surfaces pass the same descriptor set to both the
 * renderer and `structuredEnvKeys()` — one policy, no local rebuilding.
 *
 * Returns `[]` when `runtime` is undefined (catalog not yet settled, or the
 * runtime has no numeric env-var fields).
 */
export function deriveNumericDescriptors(
  runtime: AcpRuntimeCatalogEntry | undefined,
): NumericDescriptor[] {
  if (!runtime) return [];
  const ds: NumericDescriptor[] = [];
  if (runtime.maxTokensEnvVar) {
    ds.push({
      kind: "maxOutputTokens",
      currentPersistence: { kind: "envVar", key: runtime.maxTokensEnvVar },
      targetApplication: { kind: "envVar", key: runtime.maxTokensEnvVar },
      render: "control",
      value: null,
    });
  }
  if (runtime.contextLimitEnvVar) {
    ds.push({
      kind: "contextLimit",
      currentPersistence: { kind: "envVar", key: runtime.contextLimitEnvVar },
      targetApplication: { kind: "envVar", key: runtime.contextLimitEnvVar },
      render: "control",
      value: null,
    });
  }
  if (runtime.maxRoundsEnvVar) {
    ds.push({
      kind: "maxRounds",
      currentPersistence: { kind: "envVar", key: runtime.maxRoundsEnvVar },
      targetApplication: { kind: "envVar", key: runtime.maxRoundsEnvVar },
      render: "control",
      value: null,
    });
  }
  return ds;
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
    const nativeKey = runtime.thinkingEnvVar;
    const isBuzzAgent = runtime.id === "buzz-agent";
    // Legacy fallback applies only at record/persona tiers (scope "definition" or
    // "instance") — mirrors spawn's effort_tier_alias(global_tier=true) policy.
    // At global/onboarding scope, legacy is buzz-agent's native key (Delta 5) and
    // must never be consumed as Goose effort.
    const isRecordOrPersonaScope =
      scope === "definition" || scope === "instance";
    const legacyKey =
      !isBuzzAgent && isRecordOrPersonaScope
        ? BUZZ_AGENT_THINKING_EFFORT
        : null;
    const acceptedValues = isBuzzAgent
      ? null
      : (runtime.acceptedEffortValues ?? null);
    const effortAliases = isBuzzAgent ? null : (runtime.effortAliases ?? null);
    // resolveEffortFromEnv is the single policy source for effort reads —
    // normalizes, applies native-first / valid-legacy-fallback, and reports
    // which key was consumed. HarnessNativeEffortFields calls the same fn.
    const { value: effortValue, legacyConsumed } = resolveEffortFromEnv(
      config.env_vars,
      nativeKey,
      legacyKey,
      acceptedValues,
      effortAliases,
    );
    fields.push({
      kind: "effort",
      optionSource: isBuzzAgent
        ? "buzzAgentCatalog"
        : runtime.acceptedEffortValues
          ? "harnessNative"
          : "legacyProviderModelCatalog",
      currentPersistence: {
        kind: "envVar",
        key: nativeKey,
      },
      targetApplication: { kind: "envVar", key: nativeKey },
      render: "control",
      value: effortValue,
      legacyConsumedKey: legacyConsumed
        ? BUZZ_AGENT_THINKING_EFFORT
        : undefined,
    });
  } else {
    omissions.push({
      kind: "effort",
      reason:
        runtime?.id === "codex" ? "ownedByModelId" : "unsupportedByHarness",
    });
  }

  // Numeric fields — derived from the shared helper, then value-populated
  // from config. Any surface needing only the descriptor structure (without
  // saved values) calls deriveNumericDescriptors(runtime) directly.
  for (const d of deriveNumericDescriptors(runtime)) {
    fields.push({
      ...d,
      value: valueFromEnv(config, d.currentPersistence.key),
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

/**
 * Returns the env-var keys owned by the rendered descriptors on a surface.
 *
 * Pass only the descriptors that **actually render controls** on the surface —
 * the resulting key set should be used as `EnvVarsEditor.hiddenKeys` and to
 * exclude keys from baked-row generic display.
 *
 * Invariant: a key appears in the output only when a first-class control for
 * it renders on the surface — a persisted value must never have zero editors.
 *
 * Per-surface consequences (assuming standard descriptor sets):
 * - Global: effort key + numeric keys rendered by the descriptors
 * - Per-agent buzz-agent: effort key + 3 numeric keys
 * - Per-agent Goose: effort key (GOOSE_THINKING_EFFORT) + 2 numeric keys
 */
export function structuredEnvKeys(
  renderedDescriptors: AgentConfigFieldDescriptor[],
): string[] {
  const keys: string[] = [];
  for (const d of renderedDescriptors) {
    if (d.render !== "control") continue;
    if (d.kind === "effort" && d.currentPersistence.kind === "envVar") {
      keys.push(d.currentPersistence.key);
    } else if (
      d.kind === "maxOutputTokens" ||
      d.kind === "contextLimit" ||
      d.kind === "maxRounds"
    ) {
      keys.push(d.currentPersistence.key);
    }
  }
  return keys;
}

/**
 * Filters a baked-env row array to exclude keys already covered by structured
 * controls, preventing double-editing. The result is the set of baked rows
 * that the generic env-vars editor should display.
 *
 * Call with the union of always-structured keys (provider/model/effort set)
 * and numeric structured keys derived from `structuredEnvKeys()`.
 *
 * Pure — suitable for Node-layer unit tests without a component renderer.
 */
export function filterBakedGenericRows<T extends { key: string }>(
  bakedEnv: readonly T[],
  excludeKeys: ReadonlySet<string> | readonly string[],
): T[] {
  const exclude =
    excludeKeys instanceof Set ? excludeKeys : new Set(excludeKeys);
  return bakedEnv.filter((e) => !exclude.has(e.key));
}

/**
 * Returns the placeholder string for a numeric tuning input.
 *
 * When an inherited value is present, the field shows `"Inherit (<value>)"`.
 * When absent (no global setting), the field shows `"Inherit (agent default)"`.
 *
 * Pure — used by NumericTuningFields and testable without a component renderer.
 */
export function numericTuningPlaceholder(
  inheritedValue: string | null | undefined,
): string {
  return inheritedValue
    ? `Inherit (${inheritedValue})`
    : "Inherit (agent default)";
}

/**
 * Return all known native thinking-effort env keys from the runtime catalog.
 *
 * Derived from `runtime.thinkingEnvVar` declarations rather than a manually
 * maintained constant — adding a new runtime to the catalog automatically
 * participates in foreign-key stripping and transition cleanup.
 *
 * Use this in place of the static `ALL_KNOWN_EFFORT_KEYS` constant wherever
 * the runtime catalog is available.
 */
export function allKnownEffortKeys(
  runtimes: readonly Pick<AcpRuntimeCatalogEntry, "thinkingEnvVar">[],
): string[] {
  const keys = new Set<string>();
  for (const rt of runtimes) {
    if (rt.thinkingEnvVar) keys.add(rt.thinkingEnvVar);
  }
  return [...keys];
}
