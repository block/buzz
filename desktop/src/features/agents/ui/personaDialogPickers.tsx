import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export const PERSONA_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
export const PERSONA_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-muted-foreground focus:outline-hidden focus-visible:ring-0";

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

type PersonaModelOption = {
  id: string;
  label: string;
  providers?: readonly PersonaLlmProviderId[];
};

export type PersonaDropdownOption = {
  disabled?: boolean;
  label: string;
  value: string;
};

const AUTO_MODEL_OPTION: PersonaModelOption = {
  id: "",
  label: "Auto (default)",
};

const PERSONA_LLM_PROVIDER_OPTIONS: readonly PersonaModelOption[] = [
  { id: "", label: "Auto (default)" },
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "openai-compat", label: "OpenAI-compatible" },
  { id: "databricks", label: "Databricks" },
];

const PERSONA_MODEL_OPTIONS_BY_RUNTIME: Record<
  string,
  readonly PersonaModelOption[]
> = {
  goose: [
    AUTO_MODEL_OPTION,
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
      id: "gpt-5",
      label: "GPT-5",
      providers: ["databricks", "openai", "openai-compat"],
    },
    {
      id: "gpt-5-mini",
      label: "GPT-5 mini",
      providers: ["databricks", "openai", "openai-compat"],
    },
    {
      id: "gemini-2.5-pro",
      label: "Gemini 2.5 Pro",
      providers: ["databricks", "openai-compat"],
    },
    {
      id: "gemini-2.5-flash",
      label: "Gemini 2.5 Flash",
      providers: ["databricks", "openai-compat"],
    },
  ],
  "buzz-agent": [
    AUTO_MODEL_OPTION,
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
      id: "gpt-5",
      label: "GPT-5",
      providers: ["databricks", "openai", "openai-compat"],
    },
    {
      id: "gpt-5-mini",
      label: "GPT-5 mini",
      providers: ["databricks", "openai", "openai-compat"],
    },
    {
      id: "gemini-2.5-pro",
      label: "Gemini 2.5 Pro",
      providers: ["databricks", "openai-compat"],
    },
    {
      id: "gemini-2.5-flash",
      label: "Gemini 2.5 Flash",
      providers: ["databricks", "openai-compat"],
    },
  ],
  claude: [AUTO_MODEL_OPTION],
  codex: [AUTO_MODEL_OPTION],
};

export function getRuntimePersonaModelOptions(
  runtimeId: string,
): readonly PersonaModelOption[] {
  return PERSONA_MODEL_OPTIONS_BY_RUNTIME[runtimeId] ?? [AUTO_MODEL_OPTION];
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
  if (!isKnownLlmProvider(trimmedProvider)) {
    return options;
  }

  return options.filter(
    (option) =>
      option.id.length === 0 || option.providers?.includes(trimmedProvider),
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

export function getPersonaProviderOptions(
  currentProvider: string,
): readonly PersonaModelOption[] {
  const trimmedProvider = currentProvider.trim();
  if (
    trimmedProvider.length === 0 ||
    PERSONA_LLM_PROVIDER_OPTIONS.some((option) => option.id === trimmedProvider)
  ) {
    return PERSONA_LLM_PROVIDER_OPTIONS;
  }

  return [
    ...PERSONA_LLM_PROVIDER_OPTIONS,
    { id: trimmedProvider, label: `${trimmedProvider} (current)` },
  ];
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
    available.find((runtime) => runtime.id === "goose") ??
    available.find((runtime) => runtime.id === "buzz-agent") ??
    available[0] ??
    null
  );
}
