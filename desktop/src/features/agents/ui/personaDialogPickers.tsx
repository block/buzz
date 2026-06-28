import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

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
  "openai",
  "openai-compat",
] as const;

type PersonaLlmProviderId = (typeof KNOWN_LLM_PROVIDER_IDS)[number];

export type PersonaModelOption = {
  id: string;
  label: string;
  providers?: readonly PersonaLlmProviderId[];
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

const DATABRICKS_DEFAULT_MODEL_OPTION: PersonaModelOption = {
  id: "",
  label: "Databricks default model",
};

// Databricks IDs are sourced from squareup/goose-releases goose_models.json.
// `goose-claude-4-8-opus` is also the current Buzz internal build default in
// squareup/buzz-releases, though it is ahead of that registry today.
const BUZZ_AGENT_MODEL_OPTIONS: readonly PersonaModelOption[] = [
  DATABRICKS_DEFAULT_MODEL_OPTION,
  {
    id: "goose-claude-4-8-opus",
    label: "Claude Opus 4.8",
    providers: ["databricks"],
  },
  {
    id: "goose-claude-4-7-opus",
    label: "Claude Opus 4.7",
    providers: ["databricks"],
  },
  {
    id: "goose-claude-4-6-opus",
    label: "Claude Opus 4.6",
    providers: ["anthropic", "databricks"],
  },
  {
    id: "goose-claude-4-6-sonnet",
    label: "Claude Sonnet 4.6",
    providers: ["anthropic", "databricks"],
  },
  {
    id: "goose-claude-4-5-opus",
    label: "Claude Opus 4.5",
    providers: ["databricks"],
  },
  {
    id: "goose-claude-4-5-sonnet",
    label: "Claude Sonnet 4.5",
    providers: ["databricks"],
  },
  {
    id: "goose-claude-4-5-haiku",
    label: "Claude Haiku 4.5",
    providers: ["databricks"],
  },
  {
    id: "goose-gpt-5-2",
    label: "GPT-5.2",
    providers: ["databricks"],
  },
  {
    id: "databricks-gpt-5-5-pro",
    label: "GPT-5.5 Pro",
    providers: ["databricks"],
  },
  {
    id: "databricks-gpt-5-5",
    label: "GPT-5.5",
    providers: ["databricks"],
  },
  {
    id: "gpt-5.5",
    label: "GPT-5.5",
    providers: ["openai", "openai-compat"],
  },
  {
    id: "gpt-5.4",
    label: "GPT-5.4",
    providers: ["openai", "openai-compat"],
  },
  {
    id: "gpt-5.4-mini",
    label: "GPT-5.4 mini",
    providers: ["openai", "openai-compat"],
  },
  {
    id: "gpt-5.4-nano",
    label: "GPT-5.4 nano",
    providers: ["openai", "openai-compat"],
  },
  {
    id: "gpt-5",
    label: "GPT-5",
    providers: ["openai", "openai-compat"],
  },
  {
    id: "gpt-5-mini",
    label: "GPT-5 mini",
    providers: ["openai", "openai-compat"],
  },
  {
    id: "goose-gemini-3-5-flash",
    label: "Gemini 3.5 Flash",
    providers: ["databricks"],
  },
  {
    id: "goose-gemini-3-1-pro",
    label: "Gemini 3.1 Pro",
    providers: ["databricks"],
  },
  {
    id: "goose-gemini-3-1-flash-lite",
    label: "Gemini 3.1 Flash Lite",
    providers: ["databricks"],
  },
  {
    id: "goose-gemini-2-5-pro",
    label: "Gemini 2.5 Pro",
    providers: ["databricks"],
  },
  {
    id: "gemini-2.5-pro",
    label: "Gemini 2.5 Pro",
    providers: ["openai-compat"],
  },
  {
    id: "gemini-2.5-flash",
    label: "Gemini 2.5 Flash",
    providers: ["openai-compat"],
  },
];

const PERSONA_LLM_PROVIDER_OPTIONS: readonly PersonaModelOption[] = [
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "openai-compat", label: "OpenAI-compatible" },
  { id: "databricks", label: "Databricks" },
];

const PERSONA_MODEL_OPTIONS_BY_RUNTIME: Record<
  string,
  readonly PersonaModelOption[]
> = {
  goose: BUZZ_AGENT_MODEL_OPTIONS,
  "buzz-agent": BUZZ_AGENT_MODEL_OPTIONS,
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

export function getPersonaModelOptions(
  runtimeId: string,
  providerId: string | null | undefined,
): readonly PersonaModelOption[] {
  const options = getRuntimePersonaModelOptions(runtimeId);
  const trimmedProvider = providerId?.trim() ?? "";
  if (trimmedProvider.length === 0) {
    return options.filter((option) => option.id.length === 0);
  }
  if (!isKnownLlmProvider(trimmedProvider)) {
    return options;
  }

  return options.filter(
    (option) =>
      (option.id.length === 0 &&
        !providerRequiresExplicitModel(trimmedProvider)) ||
      option.providers?.includes(trimmedProvider),
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

export function getDefaultLlmProviderLabel(runtimeId: string) {
  return runtimeId === "buzz-agent" || runtimeId === "goose"
    ? "Databricks default"
    : "Default";
}

export function getPersonaProviderOptions(
  currentProvider: string,
  runtimeId: string,
): readonly PersonaModelOption[] {
  const trimmedProvider = currentProvider.trim();
  const options = [
    { id: "", label: getDefaultLlmProviderLabel(runtimeId) },
    ...PERSONA_LLM_PROVIDER_OPTIONS,
  ];
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
