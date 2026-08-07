import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import {
  BUZZ_AGENT_THINKING_EFFORT_VALUES,
  getProviderEffortConfig,
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

export type StructuredJsonEnvPersistence = {
  kind: "structuredJsonEnv";
  envKey: string;
  jsonKey: string;
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
      optionSource: "buzzAgentCatalog" | "legacyProviderModelCatalog";
      currentPersistence: EnvVarPersistence | StructuredJsonEnvPersistence;
      targetApplication:
        | { kind: "envVar"; key: string }
        | { kind: "structuredJsonEnv"; envKey: string; jsonKey: string };
      render: "control";
      value: string | null;
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
  reason: "unsupportedByHarness";
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
 * An effort field backed by the environment, in either shape a harness accepts:
 * a plain variable (`CLAUDE_CODE_EFFORT_LEVEL`, `GOOSE_THINKING_EFFORT`) or one
 * property inside a structured JSON variable (Codex's `CODEX_CONFIG`).
 *
 * Both shapes carry the *same* key in `currentPersistence` and
 * `targetApplication`: the value is stored exactly where the harness reads it,
 * so there is no translation step that can silently drop an edit.
 */
export type EffortEnvDescriptor = {
  kind: "effort";
  optionSource: "legacyProviderModelCatalog";
  currentPersistence: EnvVarPersistence | StructuredJsonEnvPersistence;
  targetApplication:
    | { kind: "envVar"; key: string }
    | { kind: "structuredJsonEnv"; envKey: string; jsonKey: string };
  render: "control";
  value: null;
  values: readonly string[];
};

/**
 * Derives an effort field for a catalog-declared environment target. The
 * renderer never needs to know a runtime ID, env name, or JSON property.
 *
 * Returns `undefined` when the runtime declares no environment target for
 * effort, which is the only reason a surface should render no control.
 */
export function deriveEffortEnvDescriptor(
  runtime: AcpRuntimeCatalogEntry | undefined,
): EffortEnvDescriptor | undefined {
  const persistence = effortEnvPersistence(runtime);
  if (!persistence) return undefined;
  return {
    kind: "effort",
    optionSource: "legacyProviderModelCatalog",
    currentPersistence: persistence,
    targetApplication: persistence,
    render: "control",
    value: null,
    values: effortValuesForRuntime(runtime?.id ?? ""),
  };
}

/**
 * The effort levels a generic (non-buzz-agent) control may offer for a harness.
 *
 * A harness that locks its provider accepts only that provider's levels, so
 * offering the full buzz-agent list would let the user pick a value the harness
 * silently drops. Claude Code is the concrete case: its CLI enum is
 * `["low","medium","high","xhigh","max"]` (plus the `med`/`ultracode` aliases),
 * and `CLAUDE_CODE_EFFORT_LEVEL=none` resolves to "unset" rather than an effort
 * — exactly the silent no-op this module exists to prevent. The Anthropic table
 * in `getProviderEffortConfig` already encodes that set.
 *
 * Harnesses whose provider is user-selectable (Goose) get the full list,
 * because the accepted set depends on a provider/model this surface does not
 * know; the per-provider narrowing happens on the surfaces that do know.
 */
function effortValuesForRuntime(runtimeId: string): readonly string[] {
  const provider = implicitEffortProvider(runtimeId);
  if (!provider) return BUZZ_AGENT_THINKING_EFFORT_VALUES;
  return getProviderEffortConfig(provider, "").validValues;
}

function effortEnvPersistence(
  runtime: AcpRuntimeCatalogEntry | undefined,
): EnvVarPersistence | StructuredJsonEnvPersistence | undefined {
  if (runtime?.thinkingConfigJsonEnvVar && runtime.thinkingConfigJsonKey) {
    return {
      kind: "structuredJsonEnv",
      envKey: runtime.thinkingConfigJsonEnvVar,
      jsonKey: runtime.thinkingConfigJsonKey,
    };
  }
  if (runtime?.thinkingEnvVar) {
    return { kind: "envVar", key: runtime.thinkingEnvVar };
  }
  return undefined;
}

/**
 * The provider whose effort table applies when a harness locks its provider and
 * therefore renders no provider control. Named here rather than in a component
 * because it is a harness-identity fact, not presentation (see AGENTS.md rule 1).
 */
export function implicitEffortProvider(runtimeId: string): string {
  if (runtimeId === "claude") return "anthropic";
  if (runtimeId === "codex") return "openai";
  return "";
}

/**
 * Read the effort value from whichever environment shape the target uses.
 *
 * `persistence` is optional so surfaces can call this before the catalog has
 * settled (or for a harness with no effort target) without branching: with no
 * target there is no stored value, so the answer is `""`.
 */
export function readEffortEnvValue(
  envVars: Record<string, string>,
  persistence: EffortEnvDescriptor["currentPersistence"] | undefined,
): string {
  if (!persistence) return "";
  return persistence.kind === "structuredJsonEnv"
    ? readStructuredJsonEnvValue(envVars, persistence)
    : (envVars[persistence.key] ?? "");
}

/**
 * Write (or clear, on `""`) the effort value in whichever environment shape the
 * target uses. Never mutates the input map.
 *
 * With no `persistence` there is nothing to write, so the map is returned
 * copied and unchanged — callers that clear effort alongside other edits stay
 * branch-free.
 */
export function updateEffortEnvValue(
  envVars: Record<string, string>,
  persistence: EffortEnvDescriptor["currentPersistence"] | undefined,
  value: string,
): Record<string, string> {
  if (!persistence) return { ...envVars };
  if (persistence.kind === "structuredJsonEnv") {
    return updateStructuredJsonEnvValue(envVars, persistence, value);
  }
  const next = { ...envVars };
  if (value) {
    next[persistence.key] = value;
  } else {
    delete next[persistence.key];
  }
  return next;
}

/** Read one string property from a descriptor-owned JSON env value. */
export function readStructuredJsonEnvValue(
  envVars: Record<string, string>,
  persistence: StructuredJsonEnvPersistence,
): string {
  try {
    const parsed: unknown = JSON.parse(envVars[persistence.envKey] ?? "{}");
    if (
      parsed !== null &&
      typeof parsed === "object" &&
      typeof (parsed as Record<string, unknown>)[persistence.jsonKey] ===
        "string"
    ) {
      return (parsed as Record<string, string>)[persistence.jsonKey];
    }
  } catch {
    // Generic env editing may have left invalid JSON; preserve it until the
    // user selects an effort rather than hiding or silently discarding it.
  }
  return "";
}

/**
 * Update one descriptor-owned JSON property without discarding unrelated
 * configuration. Blank removes only that property, deleting the env key only
 * when it becomes an empty object.
 */
export function updateStructuredJsonEnvValue(
  envVars: Record<string, string>,
  persistence: StructuredJsonEnvPersistence,
  value: string,
): Record<string, string> {
  let config: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(envVars[persistence.envKey] ?? "{}");
    if (
      parsed !== null &&
      typeof parsed === "object" &&
      !Array.isArray(parsed)
    ) {
      config = { ...(parsed as Record<string, unknown>) };
    }
  } catch {
    // Selecting a structured value replaces invalid JSON with the minimal
    // valid object required by this target.
  }
  if (value) {
    config[persistence.jsonKey] = value;
  } else {
    delete config[persistence.jsonKey];
  }
  const next = { ...envVars };
  if (Object.keys(config).length === 0) {
    delete next[persistence.envKey];
  } else {
    next[persistence.envKey] = JSON.stringify(config);
  }
  return next;
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

  const effortPersistence = effortEnvPersistence(runtime);
  if (effortPersistence) {
    fields.push({
      kind: "effort",
      optionSource:
        runtime?.id === "buzz-agent"
          ? "buzzAgentCatalog"
          : "legacyProviderModelCatalog",
      // Store the value where the harness reads it. A previous revision pinned
      // this to BUZZ_AGENT_THINKING_EFFORT for every runtime, which made the
      // Goose and Claude Code controls write a key their harness ignores.
      currentPersistence: effortPersistence,
      targetApplication: effortPersistence,
      render: "control",
      value:
        effortPersistence.kind === "envVar"
          ? valueFromEnv(config, effortPersistence.key)
          : readStructuredJsonEnvValue(config.env_vars, effortPersistence) ||
            null,
    });
  } else {
    omissions.push({ kind: "effort", reason: "unsupportedByHarness" });
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
 * - Per-agent Goose: 2 numeric keys only — Goose effort (BUZZ_AGENT_THINKING_EFFORT)
 *   stays a visible generic env row because no effort control renders per-agent
 *   for Goose (effort migration is out of scope)
 */
export function structuredEnvKeys(
  renderedDescriptors: AgentConfigFieldDescriptor[],
): string[] {
  const keys: string[] = [];
  for (const d of renderedDescriptors) {
    if (d.render !== "control") continue;
    if (d.kind === "effort") {
      if (d.currentPersistence.kind === "envVar") {
        keys.push(d.currentPersistence.key);
      } else if (d.currentPersistence.kind === "structuredJsonEnv") {
        keys.push(d.currentPersistence.envKey);
      }
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
