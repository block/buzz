/**
 * Controlled field group for global agent config (provider, model, effort, env vars).
 *
 * Used by GlobalAgentConfigSettingsCard (settings panel) and AgentDefaultsSection
 * (onboarding setup step). The parent manages load/save state; this component is
 * purely presentational and calls onConfigChange on every user edit.
 */
import * as React from "react";
import { ChevronDown } from "lucide-react";

import type { BakedEnvEntry } from "@/shared/api/tauri";
import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { EnvVarsEditor } from "@/features/agents/ui/EnvVarsEditor";
import type { InheritedEnvRow } from "@/features/agents/ui/EnvVarsEditor";
import {
  getBakedProviderInheritLabel,
  getGlobalModelFallback,
} from "@/features/agents/ui/bakedEnvHelpers";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  getPersonaProviderOptions,
  getProviderApiKeyEnvVar,
  requiredCredentialEnvKeys,
} from "@/features/agents/ui/personaDialogPickers";
import {
  AgentDropdownSelect,
  AgentModelField,
} from "@/features/agents/ui/personaProviderModelFields";
import { PersonaProviderApiKeyField } from "@/features/agents/ui/PersonaProviderApiKeyField";
import { usePersonaModelDiscovery } from "@/features/agents/ui/usePersonaModelDiscovery";
import {
  BUZZ_AGENT_THINKING_EFFORT,
  getProviderEffortConfig,
} from "@/features/agents/ui/buzzAgentConfig";
import {
  EffortSelectField,
  useEffortAutoClear,
} from "@/features/agents/ui/buzzAgentModelTuningFields";
import { Input } from "@/shared/ui/input";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";

/** Sentinel value for an unconfigured global agent config. */
export const EMPTY_GLOBAL_CONFIG: GlobalAgentConfig = {
  env_vars: {},
  provider: null,
  model: null,
  preferred_runtime: null,
};

/** Baked env keys that route to structured controls, not the generic env editor. */
const BAKED_STRUCTURED_KEYS = new Set([
  "BUZZ_AGENT_PROVIDER",
  "BUZZ_AGENT_MODEL",
  BUZZ_AGENT_THINKING_EFFORT,
]);

export type GlobalAgentConfigFieldsProps = {
  bakedEnv: BakedEnvEntry[];
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  config: GlobalAgentConfig;
  isCustomModelEditing: boolean;
  isCustomProvider: boolean;
  onConfigChange: (next: GlobalAgentConfig) => void;
  onCustomModelEditingChange: (value: boolean) => void;
  onIsCustomProviderChange: (value: boolean) => void;
  onValidityChange?: (valid: boolean) => void;
  autoSelectModelOnProviderChange?: boolean;
  disableModelSelectDuringDiscovery?: boolean;
  effortPlaceholderLabel?: string;
  effortLabel?: string;
  keepSelectedModelValueLabel?: boolean;
  modelPlaceholderLabel?: string;
  providerLabel?: string;
  requireProviderForModelAndEffort?: boolean;
  selectClassName?: string;
  showAdvancedFields?: boolean;
  showCustomModelOption?: boolean;
  showCustomProviderOption?: boolean;
  showDescriptions?: boolean;
  showEffortField?: boolean;
  showProviderPlaceholderOption?: boolean;
  showRequiredIndicators?: boolean;
  showUnavailableEffortOptions?: boolean;
  unstyled?: boolean;
  useCustomSelect?: boolean;
  useChevronSelectIcon?: boolean;
};

export function GlobalAgentConfigFields({
  bakedEnv,
  selectedRuntime,
  config,
  isCustomModelEditing,
  isCustomProvider,
  onConfigChange,
  onCustomModelEditingChange,
  onIsCustomProviderChange,
  onValidityChange,
  autoSelectModelOnProviderChange = false,
  disableModelSelectDuringDiscovery = true,
  effortPlaceholderLabel,
  effortLabel = "Thinking/effort",
  keepSelectedModelValueLabel = false,
  modelPlaceholderLabel = "Select model",
  providerLabel = "LLM provider",
  requireProviderForModelAndEffort = false,
  selectClassName,
  showAdvancedFields = true,
  showCustomModelOption = true,
  showCustomProviderOption = true,
  showDescriptions = true,
  showEffortField = true,
  showProviderPlaceholderOption = true,
  showRequiredIndicators = true,
  showUnavailableEffortOptions = true,
  unstyled = false,
  useCustomSelect = false,
  useChevronSelectIcon = false,
}: GlobalAgentConfigFieldsProps) {
  const bakedProvider = React.useMemo(
    () => bakedEnv.find((e) => e.key === "BUZZ_AGENT_PROVIDER")?.value ?? null,
    [bakedEnv],
  );
  const effectiveProvider = config.provider?.trim() || bakedProvider || "";
  const fallbackModel = React.useMemo(
    () => getGlobalModelFallback(bakedEnv, effectiveProvider, config.env_vars),
    [bakedEnv, config.env_vars, effectiveProvider],
  );
  const modelIsValid =
    (config.model?.trim().length ?? 0) > 0 || fallbackModel !== null;
  React.useEffect(() => {
    onValidityChange?.(modelIsValid);
  }, [modelIsValid, onValidityChange]);
  const bakedEffort = React.useMemo(
    () =>
      bakedEnv.find((e) => e.key === BUZZ_AGENT_THINKING_EFFORT)?.value ?? null,
    [bakedEnv],
  );
  const bakedGenericRows = React.useMemo<readonly InheritedEnvRow[]>(
    () => bakedEnv.filter((e) => !BAKED_STRUCTURED_KEYS.has(e.key)),
    [bakedEnv],
  );

  const providerValue = config.provider ?? "";
  const providerForDiscovery = isCustomProvider
    ? ""
    : providerValue || bakedProvider || "";
  const dependentFieldsDisabled =
    requireProviderForModelAndEffort &&
    providerForDiscovery.trim().length === 0;
  const credentialProvider = isCustomProvider ? "" : effectiveProvider;
  const requiredEnvKeys = requiredCredentialEnvKeys(
    "buzz-agent",
    credentialProvider,
  );
  const apiKeyEnvVar = getProviderApiKeyEnvVar(credentialProvider);
  const advancedRequiredEnvKeys = requiredEnvKeys.filter(
    (key) =>
      key !== apiKeyEnvVar && !bakedEnv.some((entry) => entry.key === key),
  );
  const apiKeyValue = apiKeyEnvVar ? (config.env_vars[apiKeyEnvVar] ?? "") : "";
  const bakedEnvKeys = React.useMemo(
    () => bakedEnv.map((entry) => entry.key),
    [bakedEnv],
  );
  const apiKeyInherited =
    apiKeyEnvVar !== null &&
    apiKeyValue.length === 0 &&
    bakedEnvKeys.includes(apiKeyEnvVar);

  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars: config.env_vars,
    isCustomProviderEditing: isCustomProvider,
    modelFieldVisible: !dependentFieldsDisabled,
    open: true,
    provider: providerForDiscovery,
    selectedRuntime,
  });

  const autoSelectedModelProviderRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!autoSelectModelOnProviderChange) return;
    const trimmedProvider = providerForDiscovery.trim();
    if (trimmedProvider.length === 0 || isCustomProvider) {
      autoSelectedModelProviderRef.current = null;
      return;
    }
    if ((config.model ?? "").trim().length > 0) return;
    if (modelDiscoveryLoading || discoveredModelOptions === null) return;
    if (autoSelectedModelProviderRef.current === trimmedProvider) return;

    const firstModel = discoveredModelOptions.find(
      (option) => option.id.trim().length > 0,
    );
    if (!firstModel) return;

    autoSelectedModelProviderRef.current = trimmedProvider;
    onCustomModelEditingChange(false);
    onConfigChange({ ...config, model: firstModel.id });
  }, [
    autoSelectModelOnProviderChange,
    config,
    discoveredModelOptions,
    isCustomProvider,
    modelDiscoveryLoading,
    onConfigChange,
    onCustomModelEditingChange,
    providerForDiscovery,
  ]);

  const currentEffortForAutoClear =
    config.env_vars[BUZZ_AGENT_THINKING_EFFORT] ?? "";
  React.useEffect(() => {
    if (!dependentFieldsDisabled) return;
    if (
      (config.model ?? "").trim().length === 0 &&
      currentEffortForAutoClear.length === 0
    ) {
      return;
    }

    const nextEnvVars = { ...config.env_vars };
    delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
    onCustomModelEditingChange(false);
    onConfigChange({ ...config, env_vars: nextEnvVars, model: null });
  }, [
    config,
    currentEffortForAutoClear,
    dependentFieldsDisabled,
    onConfigChange,
    onCustomModelEditingChange,
  ]);
  const { validValues: effortValidForAutoClear } = getProviderEffortConfig(
    config.provider ?? "",
    config.model ?? "",
  );
  useEffortAutoClear({
    currentEffort: currentEffortForAutoClear,
    effortValid: effortValidForAutoClear,
    onClear: () => {
      const nextEnvVars = { ...config.env_vars };
      delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
      onConfigChange({ ...config, env_vars: nextEnvVars });
    },
  });

  function handleProviderChange(value: string) {
    const previousApiKey = getProviderApiKeyEnvVar(effectiveProvider);
    if (value === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
      const nextEnvVars = { ...config.env_vars };
      if (previousApiKey) delete nextEnvVars[previousApiKey];
      onIsCustomProviderChange(true);
      onConfigChange({ ...config, env_vars: nextEnvVars, provider: null });
      return;
    }
    const nextProvider =
      value === AUTO_PROVIDER_DROPDOWN_VALUE || value === "" ? null : value;
    const nextApiKey = getProviderApiKeyEnvVar(
      nextProvider ?? bakedProvider ?? "",
    );
    const nextEnvVars = { ...config.env_vars };
    if (previousApiKey && previousApiKey !== nextApiKey) {
      delete nextEnvVars[previousApiKey];
    }
    const providerChanged = nextProvider !== (config.provider ?? null);

    onIsCustomProviderChange(false);
    onConfigChange({
      ...config,
      env_vars: nextEnvVars,
      provider: nextProvider,
      model:
        nextProvider === "relay-mesh"
          ? config.model || "auto"
          : autoSelectModelOnProviderChange && providerChanged
            ? null
            : config.model,
    });
  }

  function handleCustomProviderInput(value: string) {
    onConfigChange({ ...config, provider: value || null });
  }

  function handleModelChange(value: string) {
    onConfigChange({
      ...config,
      model: config.provider === "relay-mesh" ? value || "auto" : value || null,
    });
  }

  function handleEnvVarsChange(next: Record<string, string>) {
    const effort = config.env_vars[BUZZ_AGENT_THINKING_EFFORT];
    const merged =
      effort !== undefined
        ? { ...next, [BUZZ_AGENT_THINKING_EFFORT]: effort }
        : next;
    onConfigChange({ ...config, env_vars: merged });
  }

  // On internal Block builds, BUZZ_AGENT_PROVIDER is baked in and a boot
  // migration rewrites v1→v2. Hide the legacy v1 option so it is not offered
  // for new selections; OSS builds show it.
  const hideProviderIds = React.useMemo(
    () =>
      bakedEnvKeys.includes("BUZZ_AGENT_PROVIDER")
        ? BLOCK_BUILD_HIDDEN_PROVIDER_IDS
        : new Set<string>(),
    [bakedEnvKeys],
  );
  const providerOptions = getPersonaProviderOptions(
    providerValue,
    "buzz-agent",
    undefined,
    hideProviderIds,
  );
  const providerSelectValue = isCustomProvider
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : providerValue || AUTO_PROVIDER_DROPDOWN_VALUE;

  const providerZeroLabel = React.useMemo(() => {
    if (!bakedProvider) return null;
    return getBakedProviderInheritLabel(bakedProvider, providerOptions);
  }, [bakedProvider, providerOptions]);
  const compactProviderZeroLabel = React.useMemo(() => {
    if (bakedProvider) {
      return (
        providerOptions.find((option) => option.id === bakedProvider)?.label ??
        bakedProvider
      );
    }
    return "Select a provider";
  }, [bakedProvider, providerOptions]);

  const { validValues: effortValid, defaultValue: effortDefault } =
    getProviderEffortConfig(config.provider ?? "", config.model ?? "");
  const currentEffort = config.env_vars[BUZZ_AGENT_THINKING_EFFORT] ?? "";

  const fieldClassName = unstyled ? "space-y-4" : "space-y-1.5 p-3";
  const blockClassName = unstyled ? "" : "p-3";
  const fieldLabelClassName = unstyled ? "pl-3" : undefined;
  const providerDropdownOptions = [
    ...providerOptions
      .filter(
        (opt) =>
          showProviderPlaceholderOption ||
          opt.id !== "" ||
          providerSelectValue === AUTO_PROVIDER_DROPDOWN_VALUE,
      )
      .map((opt) => ({
        label:
          opt.id === ""
            ? showProviderPlaceholderOption
              ? (providerZeroLabel ?? opt.label)
              : compactProviderZeroLabel
            : opt.label,
        value: opt.id || AUTO_PROVIDER_DROPDOWN_VALUE,
      })),
    ...(showCustomProviderOption
      ? [{ label: "Custom provider…", value: CUSTOM_PROVIDER_DROPDOWN_VALUE }]
      : []),
  ];
  const providerSelect = useCustomSelect ? (
    <AgentDropdownSelect
      className={selectClassName}
      id="global-agent-provider"
      onValueChange={handleProviderChange}
      options={providerDropdownOptions}
      placeholder={
        showProviderPlaceholderOption
          ? "Select provider"
          : compactProviderZeroLabel
      }
      placeholderValue={
        !showProviderPlaceholderOption && !bakedProvider
          ? AUTO_PROVIDER_DROPDOWN_VALUE
          : undefined
      }
      testId="global-agent-provider"
      value={providerSelectValue}
    />
  ) : (
    <select
      className={cn(
        "flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs",
        useChevronSelectIcon && "appearance-none pr-10",
        selectClassName,
      )}
      id="global-agent-provider"
      onChange={(e) => handleProviderChange(e.target.value)}
      value={providerSelectValue}
    >
      {providerDropdownOptions.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );

  const content = (
    <>
      {/* Provider field */}
      <div className={fieldClassName}>
        <label
          className={cn("text-sm font-medium", fieldLabelClassName)}
          htmlFor="global-agent-provider"
        >
          {providerLabel}
        </label>
        {!useCustomSelect && useChevronSelectIcon ? (
          <div className="relative">
            {providerSelect}
            <ChevronDown
              aria-hidden="true"
              className="pointer-events-none absolute right-4 top-1/2 h-4 w-4 -translate-y-1/2 text-foreground"
            />
          </div>
        ) : (
          providerSelect
        )}
        {isCustomProvider ? (
          <Input
            aria-label="Custom global provider ID"
            autoCorrect="off"
            onChange={(e) => handleCustomProviderInput(e.target.value)}
            placeholder="Custom provider ID"
            value={providerValue}
          />
        ) : null}
      </div>

      {showAdvancedFields && apiKeyEnvVar ? (
        <div className={blockClassName}>
          <PersonaProviderApiKeyField
            disabled={false}
            inheritedLabel="Provided by this build"
            isInherited={apiKeyInherited}
            isRequired={!apiKeyInherited && apiKeyValue.length === 0}
            label={
              effectiveProvider === "anthropic"
                ? "Anthropic API Key"
                : "OpenAI API Key"
            }
            onValueChange={(value) =>
              onConfigChange({
                ...config,
                env_vars: { ...config.env_vars, [apiKeyEnvVar]: value },
              })
            }
            value={apiKeyValue}
          />
        </div>
      ) : null}

      {/* Model field */}
      <div className={showDescriptions ? fieldClassName : undefined}>
        <AgentModelField
          allowDefaultModel={fallbackModel !== null}
          defaultModelLabel={
            fallbackModel ? `Default model (${fallbackModel})` : undefined
          }
          disableSelectDuringDiscovery={disableModelSelectDuringDiscovery}
          disabled={dependentFieldsDisabled}
          discoveredModelOptions={
            dependentFieldsDisabled ? null : discoveredModelOptions
          }
          globalModel={fallbackModel ?? undefined}
          id="global-agent-model"
          isCustomModelEditing={isCustomModelEditing}
          isRequired={
            showRequiredIndicators &&
            fallbackModel === null &&
            !dependentFieldsDisabled
          }
          keepSelectedModelValueLabel={keepSelectedModelValueLabel}
          model={dependentFieldsDisabled ? "" : (config.model ?? "")}
          modelDiscoveryLoading={
            dependentFieldsDisabled ? false : modelDiscoveryLoading
          }
          modelDiscoveryStatus={
            dependentFieldsDisabled ? null : modelDiscoveryStatus
          }
          onIsCustomModelEditingChange={onCustomModelEditingChange}
          onModelChange={handleModelChange}
          placeholder={modelPlaceholderLabel}
          provider={providerForDiscovery}
          fieldClassName={unstyled ? fieldClassName : undefined}
          labelClassName={fieldLabelClassName}
          selectClassName={selectClassName}
          showCustomModelOption={showCustomModelOption}
          showStatusMessage={showDescriptions}
          testId="global-agent-model"
          useCustomSelect={useCustomSelect}
          useChevronIcon={useChevronSelectIcon}
        />
      </div>

      {/* Thinking / Effort */}
      {showEffortField ? (
        <div className={blockClassName}>
          <EffortSelectField
            currentEffort={dependentFieldsDisabled ? "" : currentEffort}
            disabled={dependentFieldsDisabled}
            emptyOptionLabel={effortPlaceholderLabel}
            effortDefault={effortDefault}
            effortValid={effortValid}
            fieldClassName={unstyled ? fieldClassName : undefined}
            htmlFor="global-agent-thinking-effort"
            inheritFallbackLabel={
              effortDefault !== null ? `Default (${effortDefault})` : undefined
            }
            inheritedEffort={bakedEffort ?? undefined}
            label={effortLabel}
            labelClassName={fieldLabelClassName}
            onChange={(value) => {
              const nextEnvVars = { ...config.env_vars };
              if (value === "") {
                delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
              } else {
                nextEnvVars[BUZZ_AGENT_THINKING_EFFORT] = value;
              }
              onConfigChange({ ...config, env_vars: nextEnvVars });
            }}
            selectClassName={selectClassName}
            showUnavailableOptions={showUnavailableEffortOptions}
            testId="global-agent-thinking-effort-select"
            useCustomSelect={useCustomSelect}
          />
        </div>
      ) : null}

      {showAdvancedFields ? (
        <>
          {/* Env vars */}
          <div className={blockClassName}>
            <EnvVarsEditor
              hiddenKeys={apiKeyEnvVar ? [apiKeyEnvVar] : []}
              inheritedRows={bakedGenericRows}
              inheritedRowsLabel="build"
              label="Environment variables"
              onChange={handleEnvVarsChange}
              requiredKeys={advancedRequiredEnvKeys}
              value={Object.fromEntries(
                Object.entries(config.env_vars).filter(
                  ([k]) => k !== BUZZ_AGENT_THINKING_EFFORT,
                ),
              )}
            />
          </div>
        </>
      ) : null}
    </>
  );

  if (unstyled) {
    return <div className="space-y-7">{content}</div>;
  }

  return <SettingsOptionGroup>{content}</SettingsOptionGroup>;
}
