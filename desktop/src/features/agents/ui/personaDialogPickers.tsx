import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { RuntimeFileConfigSubset } from "@/shared/api/tauri";

export const PERSONA_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
export const PERSONA_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-muted-foreground focus:outline-hidden focus-visible:ring-0";
export const PERSONA_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

export const AUTO_MODEL_DROPDOWN_VALUE = "__auto_model__";
export const CUSTOM_MODEL_DROPDOWN_VALUE = "__custom_model__";
export const AUTO_PROVIDER_DROPDOWN_VALUE = "__auto_provider__";
export const CUSTOM_PROVIDER_DROPDOWN_VALUE = "__custom_provider__";
export const NO_RUNTIME_DROPDOWN_VALUE = "__no_runtime__";

const KNOWN_LLM_PROVIDER_IDS = [
  "anthropic",
  "databricks",
  "databricks_v2",
  "openai",
  "openai-compat",
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

export type ProviderApiKeyConfig = {
  envVar: string;
  label: string;
  placeholder: string;
};

const DEFAULT_MODEL_OPTION: PersonaModelOption = {
  id: "",
  label: "Default model",
};

const PERSONA_LLM_PROVIDER_OPTIONS: readonly PersonaModelOption[] = [
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "openai-compat", label: "OpenAI-compatible" },
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
 * Returns the credential env-var keys that are required for a given
 * runtime + provider combination. These are the keys that must be present
 * in the agent's effective env for it to start successfully.
 *
 * Used by EnvVarsEditor to render first-class "required" rows that make the
 * gap visible before the user tries to save or start the agent.
 *
 * Mirrors the Rust `readiness::buzz_agent_requirements` /
 * `readiness::goose_requirements` logic — keep in sync.
 */
export function requiredCredentialEnvKeys(
  runtimeId: string,
  provider: string,
): readonly string[] {
  const normalizedRuntime = runtimeId.trim();
  const normalizedProvider = provider.trim().toLowerCase();

  // buzz-agent and goose both use provider-specific credentials.
  if (normalizedRuntime === "buzz-agent" || normalizedRuntime === "goose") {
    if (normalizedProvider === "anthropic") return ["ANTHROPIC_API_KEY"];
    if (normalizedProvider === "openai") return ["OPENAI_COMPAT_API_KEY"];
    if (
      normalizedProvider === "databricks" ||
      normalizedProvider === "databricks_v2"
    ) {
      // DATABRICKS_TOKEN is NOT required — OAuth PKCE is the normal path.
      return ["DATABRICKS_HOST"];
    }
  }

  // claude and codex handle auth via CLI login (not env keys) — those
  // requirements are surfaced separately via the CliLogin surface.
  return [];
}

export function isMissingRequiredDropdownField(
  field: { isRequired: boolean } | null | undefined,
  value: string,
) {
  return field?.isRequired === true && value.trim().length === 0;
}

export function runtimeSupportsLlmProviderSelection(runtimeId: string) {
  return runtimeId === "buzz-agent" || runtimeId === "goose";
}

function effectiveModelProviderForOptions(
  runtimeId: string,
  providerId: string | null | undefined,
) {
  if (
    runtimeId.trim().length > 0 &&
    !runtimeSupportsLlmProviderSelection(runtimeId)
  ) {
    return "";
  }

  return providerId?.trim() ?? "";
}

export function getPersonaModelOptions(
  runtimeId: string,
  providerId: string | null | undefined,
): readonly PersonaModelOption[] {
  const options = getRuntimePersonaModelOptions(runtimeId);
  const trimmedProvider = effectiveModelProviderForOptions(
    runtimeId,
    providerId,
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
    trimmedProvider === "openai-compat"
  );
}

export function getDefaultLlmProviderLabel(_runtimeId: string) {
  return "Default";
}

export function getPersonaProviderOptions(
  currentProvider: string,
  runtimeId: string,
): readonly PersonaModelOption[] {
  const trimmedProvider = currentProvider.trim();
  const defaultProviderOptions = [
    { id: "", label: getDefaultLlmProviderLabel(runtimeId) },
  ];
  const options = [...defaultProviderOptions, ...PERSONA_LLM_PROVIDER_OPTIONS];
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

export function getProviderApiKeyConfig(
  providerId: string,
): ProviderApiKeyConfig | null {
  switch (providerId.trim()) {
    case "anthropic":
      return {
        envVar: "ANTHROPIC_API_KEY",
        label: "Anthropic API key",
        placeholder: "sk-ant-...",
      };
    case "openai":
      return {
        envVar: "OPENAI_COMPAT_API_KEY",
        label: "OpenAI API key",
        placeholder: "sk-...",
      };
    case "openai-compat":
      return {
        envVar: "OPENAI_COMPAT_API_KEY",
        label: "OpenAI-compatible API key",
        placeholder: "sk-...",
      };
    default:
      return null;
  }
}

export function getProviderApiKeyEnvVar(providerId: string): string | null {
  return getProviderApiKeyConfig(providerId)?.envVar ?? null;
}

export function shouldClearKnownModelForSelectionScope({
  model,
  provider,
  runtime,
}: {
  model: string;
  provider: string | null | undefined;
  runtime: string;
}) {
  const runtimeOptions = getRuntimePersonaModelOptions(runtime);
  const scopedOptions = getPersonaModelOptions(runtime, provider);
  return (
    hasExactPersonaModelOption(runtimeOptions, model) &&
    !hasExactPersonaModelOption(scopedOptions, model)
  );
}

export function formatRuntimeOptionLabel(runtime: AcpRuntimeCatalogEntry) {
  const suffix =
    runtime.availability === "adapter_missing"
      ? " (adapter missing)"
      : runtime.availability === "cli_missing"
        ? " (CLI missing)"
        : runtime.availability === "not_installed"
          ? " (not installed)"
          : "";
  return `${runtime.label}${suffix}`;
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
  }
}

function runtimePreferenceSortRank(runtimeId: string) {
  switch (runtimeId) {
    case "buzz-agent":
      return 0;
    case "goose":
      return 1;
    default:
      return 2;
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
      runtimePreferenceSortRank(left.id) - runtimePreferenceSortRank(right.id);
    if (preferenceDelta !== 0) {
      return preferenceDelta;
    }

    return left.label.localeCompare(right.label);
  });
}

export function getDefaultPersonaRuntime(runtimes: AcpRuntimeCatalogEntry[]) {
  const available = runtimes.filter(
    (runtime) => runtime.availability === "available",
  );
  return (
    available.find((runtime) => runtime.id === "buzz-agent") ??
    available.find((runtime) => runtime.id === "goose") ??
    available[0] ??
    null
  );
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
 * isProviderMode / useMesh modes are NOT subject to this gate — they have
 * their own gates. Pass isProviderMode=true or useMesh=true to bypass.
 */
export function computeLocalModeGate({
  envVars,
  isProviderMode,
  model,
  provider,
  runtimeId,
  runtimeFileConfig,
  useMesh,
}: {
  envVars: Record<string, string>;
  isProviderMode: boolean;
  model: string;
  provider: string;
  runtimeId: string;
  /** Optional file-layer config for the runtime (e.g. goose config.yaml).
   *  When provided, requirements already satisfied there are silenced. */
  runtimeFileConfig?: RuntimeFileConfigSubset | null;
  useMesh: boolean;
}): {
  /** Normalized field names that are required but empty ("provider", "model"). */
  missingNormalizedFields: string[];
  /** Credential env key names that are required but missing or empty. */
  missingEnvKeys: string[];
  /** Env keys that are not set in Buzz but are satisfied in the runtime's
   *  config file (e.g. "Set in goose config"). */
  fileSatisfiedEnvKeys: string[];
  /** True when the create button may be enabled (from this gate's perspective). */
  satisfied: boolean;
} {
  if (isProviderMode || useMesh) {
    return {
      missingNormalizedFields: [],
      missingEnvKeys: [],
      fileSatisfiedEnvKeys: [],
      satisfied: true,
    };
  }

  const needsProviderSelection = runtimeSupportsLlmProviderSelection(runtimeId);

  // A normalized field is satisfied by the runtime file config when the file
  // provides the value (provider or model). The file layer silences the
  // requirement; the value is not injected into the Buzz env.
  const fileProvider = runtimeFileConfig?.provider?.trim() ?? "";
  const fileModel = runtimeFileConfig?.model?.trim() ?? "";
  const fileSatisfiedKeys = new Set(runtimeFileConfig?.satisfiedEnvKeys ?? []);

  const missingNormalizedFields: string[] = [];
  if (needsProviderSelection) {
    if (provider.trim().length === 0 && fileProvider.length === 0) {
      missingNormalizedFields.push("provider");
    }
    if (model.trim().length === 0 && fileModel.length === 0) {
      missingNormalizedFields.push("model");
    }
  }

  // Credential keys depend on the selected provider (empty provider → no keys
  // required beyond the normalized field gate above).
  // Use the file provider as fallback when the env provider is empty, so
  // credential requirements are computed correctly for file-config runtimes.
  const effectiveProviderForKeys = needsProviderSelection
    ? provider.trim() || fileProvider
    : "";
  const providerForKeys = needsProviderSelection
    ? effectiveProviderForKeys
    : "";
  const requiredKeys = requiredCredentialEnvKeys(runtimeId, providerForKeys);

  const missingEnvKeys: string[] = [];
  const fileSatisfiedEnvKeys: string[] = [];
  for (const key of requiredKeys) {
    if ((envVars[key] ?? "").length > 0) {
      // Set in Buzz env — satisfied, no action.
    } else if (fileSatisfiedKeys.has(key)) {
      // Not in Buzz env but present in the runtime config file — silenced.
      fileSatisfiedEnvKeys.push(key);
    } else {
      missingEnvKeys.push(key);
    }
  }

  return {
    missingNormalizedFields,
    missingEnvKeys,
    fileSatisfiedEnvKeys,
    satisfied:
      missingNormalizedFields.length === 0 && missingEnvKeys.length === 0,
  };
}
