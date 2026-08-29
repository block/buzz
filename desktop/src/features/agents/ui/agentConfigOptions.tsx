import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
  RuntimeSetupField,
} from "@/shared/api/types";
import { getPersonaRuntimePreferenceRank } from "../lib/resolvePersonaRuntime";
import { BUZZ_AGENT_THINKING_EFFORT } from "./buzzAgentConfig";
import type { RuntimeFileConfigSubset } from "@/shared/api/tauri";
// Dialogs import getDefaultPersonaRuntime via this re-export; lib code imports
// directly from lib/resolvePersonaRuntime.
export { getDefaultPersonaRuntime } from "../lib/resolvePersonaRuntime";

/**
 * Provider ids suppressed from the selection list on internal Block builds.
 * Databricks v1 (`"databricks"`) is boot-migrated to v2 on those builds, so
 * offering it for new selections would create a regression path.
 * OSS builds pass an empty `Set` so v1 remains visible.
 *
 * All three dialog sites that show a provider picker import this constant —
 * `AgentDefinitionDialog`, `AgentInstanceEditDialog`, and
 * `AgentDefaultsSettingsCard` — making it the single source of truth for
 * which provider ids to suppress on Block builds.
 */
export const BLOCK_BUILD_HIDDEN_PROVIDER_IDS: ReadonlySet<string> = new Set([
  "databricks",
]);

export const PERSONA_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
export const PERSONA_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-muted-foreground focus:outline-hidden focus-visible:ring-0";
export const PERSONA_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

/** Shared advanced-fields expand/collapse easing for the agent dialogs. */
export const ADVANCED_FIELDS_MOTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

export const AUTO_MODEL_DROPDOWN_VALUE = "__auto_model__";
export const CUSTOM_MODEL_DROPDOWN_VALUE = "__custom_model__";
export const AUTO_PROVIDER_DROPDOWN_VALUE = "__auto_provider__";
export const CUSTOM_PROVIDER_DROPDOWN_VALUE = "__custom_provider__";
export const NO_RUNTIME_DROPDOWN_VALUE = "__no_runtime__";

const KNOWN_LLM_PROVIDER_IDS = [
  "anthropic",
  "cline",
  "databricks",
  "databricks_v2",
  "openai",
  "openai-compat",
  "openrouter",
] as const;

type PersonaLlmProviderId = (typeof KNOWN_LLM_PROVIDER_IDS)[number];

export type PersonaModelOption = {
  id: string;
  label: string;
};

export type PersonaDropdownOption = {
  disabled?: boolean;
  label: string;
  value: string;
};

/**
 * Per-provider credential configuration.
 *
 * `requiredEnvKeys`: keys that must be present in the agent's effective env for
 *   the provider to work (surfaced as amber required rows in EnvVarsEditor).
 * `secretEnvVar` + `apiKeyLabel`: paired — either both are present or neither
 *   is. `secretEnvVar` is the env key holding the user-typed secret; clearing
 *   it when the user switches providers ensures no orphaned credentials remain.
 *   Databricks uses OAuth PKCE (no typed secret), so it carries neither field.
 *   `apiKeyLabel` is the human-readable label shown in the credential field;
 *   derived by `getProviderApiKeyLabel` — single source of truth for all UI
 *   surfaces so they never drift.
 *
 * Mirrors the Rust `readiness::buzz_agent_requirements` /
 * `readiness::goose_requirements` logic — keep in sync.
 */
export type ProviderCredentialConfig =
  | {
      requiredEnvKeys: readonly string[];
      secretEnvVar?: undefined;
      apiKeyLabel?: undefined;
    }
  | {
      requiredEnvKeys: readonly string[];
      /** The env key holding the user-typed API secret. */
      secretEnvVar: string;
      /** Display label for the credential input field, e.g. "Anthropic API Key". */
      apiKeyLabel: string;
      /**
       * Where the user issues this credential. Rendered as a link beside the
       * field so the answer to "where do I get a key / what does it bill
       * against?" lives with the vendor, not in Pkzz copy.
       */
      apiKeyUrl?: string;
    };

/**
 * Unified provider credential config table.  Single source of truth for both
 * required-key surfacing and provider-switch clearing semantics.
 */
const PROVIDER_CREDENTIAL_CONFIG: Partial<
  Record<string, ProviderCredentialConfig>
> = {
  anthropic: {
    requiredEnvKeys: ["ANTHROPIC_API_KEY"],
    secretEnvVar: "ANTHROPIC_API_KEY",
    apiKeyLabel: "Anthropic API Key",
  },
  openai: {
    requiredEnvKeys: ["OPENAI_COMPAT_API_KEY"],
    secretEnvVar: "OPENAI_COMPAT_API_KEY",
    apiKeyLabel: "OpenAI Runtime API Key",
  },
  "openai-compat": {
    requiredEnvKeys: ["OPENAI_COMPAT_API_KEY"],
    secretEnvVar: "OPENAI_COMPAT_API_KEY",
    apiKeyLabel: "OpenAI-compatible Runtime API Key",
  },
  databricks: {
    // DATABRICKS_TOKEN is NOT required — OAuth PKCE is the normal path.
    requiredEnvKeys: ["DATABRICKS_HOST"],
    // No secretEnvVar / apiKeyLabel: DATABRICKS_HOST is a URL, not a secret
    // credential, and is not cleared on provider switch (unlike API keys).
  },
  databricks_v2: {
    // DATABRICKS_TOKEN is NOT required — OAuth PKCE is the normal path.
    requiredEnvKeys: ["DATABRICKS_HOST"],
  },
  // Hyphen-alias for databricks_v2 emitted by the migration (#1686).
  "databricks-v2": {
    requiredEnvKeys: ["DATABRICKS_HOST"],
  },
  openrouter: {
    requiredEnvKeys: ["OPENROUTER_API_KEY"],
    secretEnvVar: "OPENROUTER_API_KEY",
    apiKeyLabel: "OpenRouter API Key",
  },
  // Cline exposes an OpenAI-compatible gateway
  // (https://api.cline.bot/api/v1). buzz-agent accepts CLINE_API_KEY and
  // falls back to OPENAI_COMPAT_API_KEY, so the credential machinery here
  // stays on the dedicated key. Issue keys at app.cline.bot > API Keys.
  cline: {
    requiredEnvKeys: ["CLINE_API_KEY"],
    secretEnvVar: "CLINE_API_KEY",
    apiKeyLabel: "Cline API Key",
    apiKeyUrl: "https://app.cline.bot/dashboard/account?tab=credits",
  },
};

const DEFAULT_MODEL_OPTION: PersonaModelOption = {
  id: "",
  label: "Default model",
};

export const PERSONA_LLM_PROVIDER_OPTIONS: readonly PersonaModelOption[] = [
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "openai-compat", label: "OpenAI-compatible" },
  { id: "openrouter", label: "OpenRouter" },
  { id: "cline", label: "Cline" },
  { id: "relay-mesh", label: "Pkzz shared compute" },
  { id: "databricks", label: "Databricks" },
  { id: "databricks_v2", label: "Databricks v2" },
];

const PERSONA_MODEL_OPTIONS_BY_RUNTIME: Record<
  string,
  readonly PersonaModelOption[]
> = {
  goose: [DEFAULT_MODEL_OPTION],
  "buzz-agent": [DEFAULT_MODEL_OPTION],
  claude: [DEFAULT_MODEL_OPTION],
  codex: [DEFAULT_MODEL_OPTION],
};

export function getRuntimePersonaModelOptions(
  runtimeId: string,
): readonly PersonaModelOption[] {
  return PERSONA_MODEL_OPTIONS_BY_RUNTIME[runtimeId] ?? [DEFAULT_MODEL_OPTION];
}

function isKnownLlmProvider(
  providerId: string,
): providerId is PersonaLlmProviderId {
  return (KNOWN_LLM_PROVIDER_IDS as readonly string[]).includes(providerId);
}

/**
 * Required credential env keys for the given runtime + provider combination.
 * Derived from PROVIDER_CREDENTIAL_CONFIG — single source of truth.
 *
 * buzz-agent and goose use provider-specific credentials; claude and codex
 * handle auth via CLI login (surfaced separately via the CliLogin surface).
 */
/**
 * Where a provider's credential is issued, when the vendor has a self-serve
 * page. Undefined for providers without one.
 */
export function providerApiKeyUrl(
  provider: string | null | undefined,
): string | undefined {
  if (!provider || !isKnownLlmProvider(provider)) return undefined;
  const config = PROVIDER_CREDENTIAL_CONFIG[provider];
  if (!config) return undefined;
  return "apiKeyUrl" in config ? config.apiKeyUrl : undefined;
}

export function requiredCredentialEnvKeys(
  runtimeId: string,
  provider: string,
  setupFields: readonly RuntimeSetupField[] = [],
): readonly string[] {
  const setupKeys = setupFields
    .filter((field) => field.required)
    .map((field) => field.envKey);
  const normalizedRuntime = runtimeId.trim();
  if (normalizedRuntime !== "buzz-agent" && normalizedRuntime !== "goose") {
    return setupKeys;
  }
  const config = PROVIDER_CREDENTIAL_CONFIG[provider.trim().toLowerCase()];
  return [...new Set([...setupKeys, ...(config?.requiredEnvKeys ?? [])])];
}

/** Validate one configured value using only Rust-owned catalog metadata. */
export function isRuntimeSetupFieldValueValid(
  field: RuntimeSetupField,
  value: string | null | undefined,
): boolean {
  const trimmed = value?.trim() ?? "";
  if (!field.required && trimmed.length === 0) return true;
  if (trimmed.length === 0) return false;

  switch (field.validation) {
    case "tailnet_https_origin": {
      try {
        const parsed = new URL(trimmed);
        const hostname = parsed.hostname.toLowerCase();
        const tailnetPrefix = hostname.endsWith(".ts.net")
          ? hostname.slice(0, -".ts.net".length)
          : "";
        return (
          parsed.protocol === "https:" &&
          tailnetPrefix.length > 0 &&
          !tailnetPrefix.endsWith(".") &&
          parsed.username.length === 0 &&
          parsed.password.length === 0 &&
          parsed.pathname === "/" &&
          parsed.search.length === 0 &&
          parsed.hash.length === 0
        );
      } catch {
        return false;
      }
    }
    case "app_password":
      return trimmed.length >= 32;
  }
}

export function isMissingRequiredDropdownField(
  field: { isRequired: boolean } | null | undefined,
  value: string,
) {
  return field?.isRequired === true && value.trim().length === 0;
}

/** Catalog-projected capability: non-null/non-empty `providerEnvVar`. */
export function runtimeSupportsLlmProviderSelection(
  providerEnvVar: string | null | undefined,
) {
  return typeof providerEnvVar === "string" && providerEnvVar.trim().length > 0;
}

/** Clears values whose meaning or support changes with the selected harness. */
export function resetConfigForHarnessChange(
  config: GlobalAgentConfig,
  runtimeId: string,
  providerEnvVar?: string | null,
): GlobalAgentConfig {
  const nextEnvVars = { ...config.env_vars };
  delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];

  return {
    ...config,
    env_vars: nextEnvVars,
    model: null,
    preferred_runtime: runtimeId || null,
    provider:
      runtimeSupportsLlmProviderSelection(providerEnvVar) &&
      config.provider !== "relay-mesh"
        ? config.provider
        : null,
  };
}

function effectiveModelProviderForOptions(
  runtimeId: string,
  providerId: string | null | undefined,
  providerEnvVar?: string | null,
) {
  if (
    runtimeId.trim().length > 0 &&
    !runtimeSupportsLlmProviderSelection(providerEnvVar)
  ) {
    return "";
  }

  return providerId?.trim() ?? "";
}

export function getPersonaModelOptions(
  runtimeId: string,
  providerId: string | null | undefined,
  providerEnvVar?: string | null,
): readonly PersonaModelOption[] {
  const options = getRuntimePersonaModelOptions(runtimeId);
  const trimmedProvider = effectiveModelProviderForOptions(
    runtimeId,
    providerId,
    providerEnvVar,
  );
  if (trimmedProvider.length === 0) {
    return options.filter((option) => option.id.length === 0);
  }
  if (!isKnownLlmProvider(trimmedProvider)) {
    return options;
  }

  return options.filter(
    (option) =>
      option.id.length === 0 && !providerRequiresExplicitModel(trimmedProvider),
  );
}

function hasExactPersonaModelOption(
  options: readonly PersonaModelOption[],
  modelId: string,
) {
  const trimmedModel = modelId.trim();
  return (
    trimmedModel.length > 0 &&
    options.some((option) => option.id === trimmedModel)
  );
}

export function hasPersonaModelOption(
  options: readonly PersonaModelOption[],
  modelId: string,
) {
  const trimmedModel = modelId.trim();
  return (
    trimmedModel.length === 0 ||
    options.some((option) => option.id === trimmedModel)
  );
}

export function getModelSelectValue({
  isCustomModelEditing,
  isModelCustom,
  model,
}: {
  isCustomModelEditing: boolean;
  isModelCustom: boolean;
  model: string;
}) {
  if (isCustomModelEditing || isModelCustom) {
    return CUSTOM_MODEL_DROPDOWN_VALUE;
  }

  return model.trim() || AUTO_MODEL_DROPDOWN_VALUE;
}

export function providerRequiresExplicitModel(
  providerId: string | null | undefined,
) {
  const trimmedProvider = providerId?.trim() ?? "";
  return (
    trimmedProvider === "anthropic" ||
    trimmedProvider === "openai" ||
    trimmedProvider === "openai-compat" ||
    trimmedProvider === "openrouter"
  );
}

export function providerDisplayLabel(providerId: string) {
  const trimmedProvider = providerId.trim();
  return trimmedProvider === "relay-mesh"
    ? "Pkzz shared compute"
    : trimmedProvider;
}

export function getDefaultLlmProviderLabel(
  _runtimeId: string,
  globalProvider?: string,
) {
  const trimmedGlobal = (globalProvider ?? "").trim();
  return trimmedGlobal
    ? `Use agent defaults (${providerDisplayLabel(trimmedGlobal)})`
    : "Select a provider\u2026";
}

/** Returns the zero-value model option label.
 *
 * When a global model is configured, the empty-model option reads
 * `Use agent defaults (<model>)` so users can see which model will run.
 * Otherwise falls back to the generic `"Default model"` placeholder.
 */
export function getDefaultLlmModelLabel(globalModel?: string) {
  const trimmedGlobal = (globalModel ?? "").trim();
  return trimmedGlobal
    ? `Use agent defaults (${trimmedGlobal})`
    : "Default model";
}

/**
 * Builds the base model dropdown options for the template dialog
 * (`AgentDefinitionDialog`), applying the global-model inherit-option guard.
 *
 * Explicit-model providers (e.g. anthropic) have their zero-value option
 * filtered out by `getPersonaModelOptions`, so a relabel-only map would never
 * produce the `Use agent defaults (<model>)` entry.  This helper prepends
 * it when `globalModel` is non-empty AND no zero-value option already exists,
 * making the inherited global model visible and selectable in the dropdown.
 *
 * GUARD: prepend only when `globalModel.trim()` is non-empty — if no global
 * model is set, an explicit-model provider must still block Save (no empty
 * inherit entry that bypasses the model requirement).
 */
export function buildTemplateModelDropdownOptions(
  modelOptions: readonly PersonaModelOption[],
  inheritedModel: string,
  inheritedModelLabel = getDefaultLlmModelLabel(inheritedModel),
): PersonaDropdownOption[] {
  const trimmedInheritedModel = inheritedModel.trim();
  const hasZeroValue = modelOptions.some((o) => o.id === "");
  const base: readonly PersonaModelOption[] =
    !hasZeroValue && trimmedInheritedModel.length > 0
      ? [{ id: "", label: inheritedModelLabel }, ...modelOptions]
      : modelOptions;
  return base.map((option) => ({
    label: option.id === "" ? inheritedModelLabel : option.label,
    value: option.id || AUTO_MODEL_DROPDOWN_VALUE,
  }));
}

/**
 * Build the provider dropdown options for a persona/instance dialog.
 *
 * `providerEnvVar` is the runtime-catalog capability that scopes named
 * providers to harnesses that implement them. Cline is a native buzz-agent
 * provider, so it is offered only when the catalog advertises
 * `BUZZ_AGENT_PROVIDER`; saved values on other runtimes still receive the
 * `(current)` tail rather than disappearing.
 *
 * `hideProviderIds` suppresses specific provider ids from the scoped list
 * while still preserving the `(current)` tail-append for saved values that
 * are in the hidden set — so an agent already persisted with a hidden
 * provider continues to render its current value, while the hidden option is
 * not offered for new selections.
 *
 * Internal Block builds pass `BLOCK_BUILD_HIDDEN_PROVIDER_IDS` to hide the
 * legacy Databricks v1 option (the boot migration rewrites v1→v2 on those
 * builds). OSS builds pass an empty Set so v1 remains visible.
 */
export function getPersonaProviderOptions(
  currentProvider: string,
  runtimeId: string,
  globalProvider?: string,
  hideProviderIds?: ReadonlySet<string>,
  providerEnvVar?: string | null,
): readonly PersonaModelOption[] {
  const trimmedProvider = currentProvider.trim();
  const defaultProviderOptions = [
    { id: "", label: getDefaultLlmProviderLabel(runtimeId, globalProvider) },
  ];
  const runtimeScopedOptions =
    providerEnvVar === "BUZZ_AGENT_PROVIDER"
      ? PERSONA_LLM_PROVIDER_OPTIONS
      : PERSONA_LLM_PROVIDER_OPTIONS.filter((option) => option.id !== "cline");
  const filteredOptions = hideProviderIds?.size
    ? runtimeScopedOptions.filter((option) => !hideProviderIds.has(option.id))
    : runtimeScopedOptions;
  const options = [...defaultProviderOptions, ...filteredOptions];
  if (
    trimmedProvider.length === 0 ||
    options.some((option) => option.id === trimmedProvider)
  ) {
    return options;
  }

  return [
    ...options,
    { id: trimmedProvider, label: `${trimmedProvider} (current)` },
  ];
}

/**
 * Returns the secret credential env var for the provider, if any.
 * Derived from PROVIDER_CREDENTIAL_CONFIG.secretEnvVar.
 */
export function getProviderApiKeyEnvVar(providerId: string): string | null {
  return (
    PROVIDER_CREDENTIAL_CONFIG[providerId.trim().toLowerCase()]?.secretEnvVar ??
    null
  );
}

/**
 * Returns the display label for the provider's API key field, if any.
 * Derived from PROVIDER_CREDENTIAL_CONFIG.apiKeyLabel — single source of truth
 * for all credential field labels so every surface stays in sync.
 *
 * Returns null when the provider has no typed-secret credential (e.g.,
 * Databricks, which uses OAuth PKCE).
 */
export function getProviderApiKeyLabel(providerId: string): string | null {
  return (
    PROVIDER_CREDENTIAL_CONFIG[providerId.trim().toLowerCase()]?.apiKeyLabel ??
    null
  );
}

/**
 * Muted contextual hint for the `OPENAI_API_KEY` row in env editors.
 * Pass as `keyAnnotations` to every `EnvVarsEditor` that may surface this key
 * (Agent Defaults, agent edit dialog, persona definition dialog).  Exported
 * so the constant is defined once and never duplicated across surfaces.
 */
export const CARD_MINT_KEY_ANNOTATIONS: Readonly<Record<string, string>> = {
  OPENAI_API_KEY: "Used for minting agent trading cards",
};

export function shouldClearKnownModelForSelectionScope({
  model,
  provider,
  providerEnvVar,
  runtime,
}: {
  model: string;
  provider: string | null | undefined;
  providerEnvVar?: string | null;
  runtime: string;
}) {
  const runtimeOptions = getRuntimePersonaModelOptions(runtime);
  const scopedOptions = getPersonaModelOptions(
    runtime,
    provider,
    providerEnvVar,
  );
  return (
    hasExactPersonaModelOption(runtimeOptions, model) &&
    !hasExactPersonaModelOption(scopedOptions, model)
  );
}

export function formatRuntimeOptionLabel(runtime: AcpRuntimeCatalogEntry) {
  const suffix =
    runtime.availability === "adapter_missing"
      ? " (adapter missing)"
      : runtime.availability === "adapter_outdated"
        ? " (adapter outdated)"
        : runtime.availability === "cli_missing"
          ? " (CLI missing)"
          : runtime.availability === "not_installed"
            ? " (not installed)"
            : runtime.runtimeReadiness === "configuration_required"
              ? " (setup required)"
              : "";
  return `${runtime.label}${suffix}`;
}

/**
 * A runtime can be newly persisted only when the catalog has both resolved a
 * runnable command and confirmed that its required setup is complete.
 */
export function isRuntimeReadyForNewSelection(
  runtime:
    | Pick<AcpRuntimeCatalogEntry, "availability" | "runtimeReadiness">
    | null
    | undefined,
) {
  return (
    runtime?.availability === "available" &&
    runtime.runtimeReadiness === "ready"
  );
}

/** A user may select setup-required runtimes only when the catalog supplies
 * the first-class fields whose save gate can make that selection runnable. */
export function isRuntimeSelectableForConfiguration(
  runtime:
    | (Pick<AcpRuntimeCatalogEntry, "availability" | "runtimeReadiness"> & {
        setupFields?: readonly RuntimeSetupField[];
      })
    | null
    | undefined,
) {
  return (
    isRuntimeReadyForNewSelection(runtime) ||
    (runtime?.availability === "available" &&
      runtime.runtimeReadiness === "configuration_required" &&
      (runtime.setupFields?.length ?? 0) > 0)
  );
}

/** Prevent a display-only fallback from becoming an implicit saved preference. */
export function shouldPersistImplicitRuntimePreference(
  preferredRuntimeId: string | null | undefined,
  runtime: AcpRuntimeCatalogEntry | null | undefined,
) {
  return !preferredRuntimeId && isRuntimeReadyForNewSelection(runtime);
}

export function buildPersonaRuntimeDropdownOptions({
  defaultRuntimeId,
  isCreateMode,
  runtime,
  runtimes,
  runtimesLoading,
}: {
  defaultRuntimeId?: string;
  isCreateMode: boolean;
  runtime: string;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
}): {
  blankRuntimeOptionLabel: string;
  runtimeDropdownOptions: PersonaDropdownOption[];
} {
  const blankRuntimeOptionLabel = runtimesLoading
    ? "Loading harnesses..."
    : isCreateMode
      ? "Choose a harness"
      : "No preference (use app default)";
  const runtimeDropdownOptions: PersonaDropdownOption[] = [
    ...(!isCreateMode
      ? [
          {
            label: blankRuntimeOptionLabel,
            value: NO_RUNTIME_DROPDOWN_VALUE,
          },
        ]
      : []),
    ...sortPersonaRuntimes(runtimes).map((candidate) => ({
      disabled:
        (isCreateMode &&
          defaultRuntimeId !== undefined &&
          candidate.availability !== "available") ||
        ((isCreateMode || candidate.id !== runtime.trim()) &&
          candidate.availability === "available" &&
          !isRuntimeSelectableForConfiguration(candidate)),
      label: `${formatRuntimeOptionLabel(candidate)}${
        isCreateMode && candidate.id === defaultRuntimeId ? " (default)" : ""
      }`,
      value: candidate.id,
    })),
  ];
  const currentRuntime = runtime.trim();
  if (
    currentRuntime.length > 0 &&
    !runtimeDropdownOptions.some((option) => option.value === currentRuntime)
  ) {
    runtimeDropdownOptions.push({
      label: `${currentRuntime} (current)`,
      value: currentRuntime,
    });
  }
  return { blankRuntimeOptionLabel, runtimeDropdownOptions };
}

function runtimeAvailabilitySortRank(
  availability: AcpRuntimeCatalogEntry["availability"],
) {
  switch (availability) {
    case "available":
      return 0;
    case "cli_missing":
      return 1;
    case "not_installed":
      return 2;
    case "adapter_missing":
      return 3;
    case "adapter_outdated":
      return 3;
  }
}

export function sortPersonaRuntimes(
  runtimes: readonly AcpRuntimeCatalogEntry[],
) {
  return [...runtimes].sort((left, right) => {
    const availabilityDelta =
      runtimeAvailabilitySortRank(left.availability) -
      runtimeAvailabilitySortRank(right.availability);
    if (availabilityDelta !== 0) {
      return availabilityDelta;
    }

    const preferenceDelta =
      getPersonaRuntimePreferenceRank(left.id) -
      getPersonaRuntimePreferenceRank(right.id);
    if (preferenceDelta !== 0) {
      return preferenceDelta;
    }

    return left.label.localeCompare(right.label);
  });
}

/**
 * Returns true when `key` is satisfied at the global layer AND the agent-local
 * `envVars` does NOT explicitly shadow it with an empty string.
 *
 * Matches backend semantics: agent env.extend() overwrites global, so an
 * agent-local value of "" makes the effective value empty → key is missing.
 * A key absent from `envVars` entirely leaves the global value intact.
 *
 * Used by both `computeLocalModeGate` (create dialog) and
 * `useRequiredCredentialState` (edit dialog) so the two gates cannot drift.
 */
export function isGloballySatisfiedCredentialKey(
  key: string,
  globalEnvVars: Record<string, string> | undefined,
  envVars: Record<string, string>,
  setupField?: RuntimeSetupField,
): boolean {
  const globalValue = globalEnvVars?.[key] ?? "";
  const globalIsValid = setupField
    ? isRuntimeSetupFieldValueValid(setupField, globalValue)
    : globalValue.length > 0;
  if (!globalIsValid) return false;
  // Agent-local "" explicitly shadows the global — effective value is empty.
  const agentExplicitlyClearedKey =
    key in envVars &&
    (setupField
      ? !isRuntimeSetupFieldValueValid(setupField, envVars[key])
      : (envVars[key] ?? "").length === 0);
  return !agentExplicitlyClearedKey;
}

/**
 * Filter a required-key list down to those satisfied by the baked build env.
 *
 * A key is baked-satisfied when the agent has no local value for it AND the
 * baked env (compile-time, Block-internal builds) contains it. This mirrors the
 * backend readiness gate Layer 1 (`resolve_effective_agent_env`) so the dialogs
 * don't surface a spurious "Required" badge for keys that are already baked in.
 *
 * OSS builds have an empty baked env, so this always returns `[]` there —
 * OSS behavior is unchanged.
 *
 * **UX asymmetry:** baked-satisfied keys are FULLY silenced — no amber Required
 * row, no "Set in config" info row. This differs from file-satisfied keys, which
 * render an info row ("Set in goose config"). Baked env is invisible
 * infrastructure; surfacing it would be noise for users.
 *
 * **Precedence:** agent-local > baked > global > file for satisfaction. An
 * explicit local empty string is still an agent-local override, so it must NOT
 * fall through to the baked layer.
 */
export function getBakedSatisfiedEnvKeys(
  requiredKeys: readonly string[],
  envVars: Record<string, string>,
  bakedEnvKeys: readonly string[] | undefined,
): string[] {
  if (!bakedEnvKeys || bakedEnvKeys.length === 0) return [];
  const bakedSet = new Set(bakedEnvKeys);
  return requiredKeys.filter((key) => !(key in envVars) && bakedSet.has(key));
}

/**
 * Pure local-mode readiness gate for Create (no existing agent, no config
 * surface query). Returns the missing normalized fields (provider, model) and
 * the missing credential env keys so the caller can derive `canSubmit`,
 * field `isRequired`, and `EnvVarsEditor.requiredKeys` from the same source.
 *
 * Two classes of required field for provider-selection runtimes (buzz-agent,
 * goose) — both required unconditionally per readiness.rs:
 *   1. Normalized fields: provider + model (empty string = NotReady)
 *   2. Credential env keys: provider-specific (e.g. ANTHROPIC_API_KEY)
 *
 * Provider mode is not subject to this gate because it has its own readiness
 * checks. Pass `isProviderMode=true` to bypass.
 */
export function computeLocalModeGate({
  bakedEnvKeys,
  envVars,
  globalEnvVars = {},
  globalProvider = "",
  globalModel = "",
  isProviderMode,
  model,
  provider,
  runtimeId,
  providerEnvVar,
  setupFields = [],
  setupSatisfiedEnvKeys = [],
  runtimeFileConfig,
}: {
  /** Optional baked build env key names (Block-internal builds only).
   *  When provided, requirements already covered by the baked env are silenced,
   *  mirroring `resolve_effective_agent_env` Layer 1 in the backend readiness
   *  gate. Absent (or empty) on OSS builds — existing call sites are unaffected. */
  bakedEnvKeys?: readonly string[];
  envVars: Record<string, string>;
  /**
   * Global agent config env vars. Required credential keys satisfied here
   * are excluded from `missingEnvKeys` so global config silences the gate.
   */
  globalEnvVars?: Record<string, string>;
  /**
   * Global fallback provider. When the agent's own provider is empty but a
   * global provider is set, the provider normalized-field gate is satisfied.
   */
  globalProvider?: string;
  /**
   * Global fallback model. When the agent's own model is empty but a global
   * model is set, the model normalized-field gate is satisfied.
   */
  globalModel?: string;
  isProviderMode: boolean;
  model: string;
  provider: string;
  runtimeId: string;
  /** Catalog-projected provider env var for the selected runtime. */
  providerEnvVar?: string | null;
  /** Rust-catalog-owned setup fields for the selected runtime. */
  setupFields?: readonly RuntimeSetupField[];
  /** Key names confirmed present in native secure storage; values never cross IPC. */
  setupSatisfiedEnvKeys?: readonly string[];
  /** Optional file-layer config for the runtime (e.g. goose config.yaml).
   *  When provided, requirements already satisfied there are silenced. */
  runtimeFileConfig?: RuntimeFileConfigSubset | null;
}): {
  /** Normalized field names that are required but empty ("provider", "model"). */
  missingNormalizedFields: string[];
  /**
   * Credential env key names that are required but not yet supplied in the
   * agent-local or global env (gate state — drives the readiness badge).
   * A key is removed from this list as soon as ANY env value provides it.
   */
  missingEnvKeys: string[];
  /**
   * Full list of credential env keys that need a locked amber row in
   * EnvVarsEditor — uses the effective provider so an agent inheriting a
   * global provider shows the correct rows. Excludes keys already satisfied
   * by global defaults or the runtime config file (those are shown
   * differently or not at all). Includes locally-filled keys so the locked
   * row remains stable while the user types a value.
   */
  requiredEnvKeys: string[];
  /** Env keys that are not set in Pkzz but are satisfied in the runtime's
   *  config file (e.g. "Set in goose config"). */
  fileSatisfiedEnvKeys: string[];
  /** True when the create button may be enabled (from this gate's perspective). */
  satisfied: boolean;
} {
  if (isProviderMode) {
    return {
      missingNormalizedFields: [],
      missingEnvKeys: [],
      requiredEnvKeys: [],
      fileSatisfiedEnvKeys: [],
      satisfied: true,
    };
  }

  const needsProviderSelection =
    runtimeSupportsLlmProviderSelection(providerEnvVar);

  // File-layer values for goose-style runtimes. These silence requirements
  // when the runtime config file provides the value — the file layer is the
  // lowest precedence fallback: env → global → file.
  const fileProvider = runtimeFileConfig?.provider?.trim() ?? "";
  const fileModel = runtimeFileConfig?.model?.trim() ?? "";
  const fileSatisfiedKeys = new Set(runtimeFileConfig?.satisfiedEnvKeys ?? []);

  // Effective provider/model: agent value → global fallback → file fallback.
  const effectiveProvider =
    provider.trim() || (globalProvider ?? "").trim() || fileProvider;
  const effectiveModel =
    model.trim() || (globalModel ?? "").trim() || fileModel;

  const missingNormalizedFields: string[] = [];
  if (needsProviderSelection) {
    if (effectiveProvider.length === 0)
      missingNormalizedFields.push("provider");
    if (effectiveModel.length === 0) missingNormalizedFields.push("model");
  }

  // Credential keys depend on the selected provider (empty provider → no keys
  // required beyond the normalized field gate above).
  // Use the effective provider (env → global → file) so credential
  // requirements are computed correctly for all config sources.
  const providerForKeys = needsProviderSelection ? effectiveProvider : "";
  const requiredKeys = requiredCredentialEnvKeys(
    runtimeId,
    providerForKeys,
    setupFields,
  );
  const setupFieldsByKey = new Map(
    setupFields.map((field) => [field.envKey, field]),
  );
  const setupSatisfiedSet = new Set(setupSatisfiedEnvKeys);

  // Keys satisfied by the baked build env (Block-internal builds only).
  const bakedSatisfiedSet = new Set(
    getBakedSatisfiedEnvKeys(requiredKeys, envVars, bakedEnvKeys),
  );

  const missingEnvKeys: string[] = [];
  const fileSatisfiedEnvKeys: string[] = [];
  // requiredEnvKeys: the full locked-row list for EnvVarsEditor. Includes
  // locally-filled keys so the amber row stays stable while the user types.
  // Excludes keys satisfied by global defaults (no locked row needed — the
  // key is already set) or by the runtime config file (shown differently).
  const requiredEnvKeys: string[] = [];
  for (const key of requiredKeys) {
    const agentValue = envVars[key] ?? "";
    const setupField = setupFieldsByKey.get(key);
    if (setupField) {
      // Remote connection values are intentionally agent-scoped. Global,
      // baked, and runtime-file layers fan out too broadly for bearer tokens.
      requiredEnvKeys.push(key);
      const securelyStored = !(key in envVars) && setupSatisfiedSet.has(key);
      if (
        !securelyStored &&
        !isRuntimeSetupFieldValueValid(setupField, agentValue)
      ) {
        missingEnvKeys.push(key);
      }
      continue;
    }
    if (isGloballySatisfiedCredentialKey(key, globalEnvVars, envVars)) {
      // Globally satisfied and not shadowed by an explicit local empty override —
      // not a missing key, and no locked row needed.
    } else if (bakedSatisfiedSet.has(key)) {
      // Not in global env but covered by the baked build env — silenced.
      // Don't add to fileSatisfiedEnvKeys; baked keys produce no info row.
    } else if (!(key in envVars) && fileSatisfiedKeys.has(key)) {
      // No higher-priority local override and present in the runtime config file.
      fileSatisfiedEnvKeys.push(key);
    } else {
      // Key needs a locked amber row in EnvVarsEditor (whether or not the
      // agent-local value is already filled — keep the row stable).
      requiredEnvKeys.push(key);
      if (agentValue.length === 0) {
        // Not filled anywhere — also surfaces as missing for gate state.
        missingEnvKeys.push(key);
      }
    }
  }

  return {
    missingNormalizedFields,
    missingEnvKeys,
    requiredEnvKeys,
    fileSatisfiedEnvKeys,
    satisfied:
      missingNormalizedFields.length === 0 && missingEnvKeys.length === 0,
  };
}
