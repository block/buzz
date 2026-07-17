/**
 * Controlled field group for global agent config (provider, model, effort, env vars).
 *
 * Used by GlobalAgentConfigSettingsCard (settings panel) and AgentDefaultsSection
 * (onboarding setup step). The parent manages load/save state; this component is
 * purely presentational and calls onConfigChange on every user edit.
 */
import * as React from "react";

import type { BakedEnvEntry } from "@/shared/api/tauri";
import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
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
import { AgentModelField } from "@/features/agents/ui/personaProviderModelFields";
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
};

/** Baked env keys that route to structured controls, not the generic env editor. */
const BAKED_STRUCTURED_KEYS = new Set([
  "BUZZ_AGENT_PROVIDER",
  "BUZZ_AGENT_MODEL",
  BUZZ_AGENT_THINKING_EFFORT,
]);

export type GlobalAgentConfigFieldsProps = {
  bakedEnv: BakedEnvEntry[];
  buzzAgentRuntime: AcpRuntimeCatalogEntry | undefined;
  config: GlobalAgentConfig;
  isCustomModelEditing: boolean;
  isCustomProvider: boolean;
  onConfigChange: (next: GlobalAgentConfig) => void;
  onCustomModelEditingChange: (value: boolean) => void;
  onIsCustomProviderChange: (value: boolean) => void;
  onValidityChange?: (valid: boolean) => void;
};

export function GlobalAgentConfigFields({
  bakedEnv,
  buzzAgentRuntime,
  config,
  isCustomModelEditing,
  isCustomProvider,
  onConfigChange,
  onCustomModelEditingChange,
  onIsCustomProviderChange,
  onValidityChange,
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
  const providerForDiscovery = isCustomProvider ? "" : providerValue;
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
    modelFieldVisible: true,
    open: true,
    provider: providerForDiscovery,
    selectedRuntime: buzzAgentRuntime,
  });

  const currentEffortForAutoClear =
    config.env_vars[BUZZ_AGENT_THINKING_EFFORT] ?? "";
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

    onIsCustomProviderChange(false);
    onConfigChange({
      ...config,
      env_vars: nextEnvVars,
      provider: nextProvider,
      model:
        nextProvider === "relay-mesh" ? config.model || "auto" : config.model,
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

  const { validValues: effortValid, defaultValue: effortDefault } =
    getProviderEffortConfig(config.provider ?? "", config.model ?? "");
  const currentEffort = config.env_vars[BUZZ_AGENT_THINKING_EFFORT] ?? "";

  return (
    <SettingsOptionGroup>
      {/* Provider field */}
      <div className="space-y-1.5 p-3">
        <label className="text-sm font-medium" htmlFor="global-agent-provider">
          Default LLM provider
        </label>
        <select
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
          id="global-agent-provider"
          onChange={(e) => handleProviderChange(e.target.value)}
          value={providerSelectValue}
        >
          {providerOptions.map((opt) => (
            <option key={opt.id} value={opt.id || AUTO_PROVIDER_DROPDOWN_VALUE}>
              {opt.id === "" ? (providerZeroLabel ?? opt.label) : opt.label}
            </option>
          ))}
          <option value={CUSTOM_PROVIDER_DROPDOWN_VALUE}>
            Custom provider…
          </option>
        </select>
        {isCustomProvider ? (
          <Input
            aria-label="Custom global provider ID"
            autoCorrect="off"
            onChange={(e) => handleCustomProviderInput(e.target.value)}
            placeholder="Custom provider ID"
            value={providerValue}
          />
        ) : null}
        <p className="text-xs text-muted-foreground">
          Applies to all agents that don't have a per-agent provider set.
        </p>
      </div>

      {apiKeyEnvVar ? (
        <div className="p-3">
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
      <div className="space-y-1.5 p-3">
        <AgentModelField
          allowDefaultModel={fallbackModel !== null}
          defaultModelLabel={
            fallbackModel ? `Default model (${fallbackModel})` : undefined
          }
          disabled={false}
          discoveredModelOptions={discoveredModelOptions}
          globalModel={fallbackModel ?? undefined}
          id="global-agent-model"
          isCustomModelEditing={isCustomModelEditing}
          isRequired={fallbackModel === null}
          model={config.model ?? ""}
          modelDiscoveryLoading={modelDiscoveryLoading}
          modelDiscoveryStatus={modelDiscoveryStatus}
          onIsCustomModelEditingChange={onCustomModelEditingChange}
          onModelChange={handleModelChange}
          provider={providerForDiscovery}
        />
        <p className="text-xs text-muted-foreground">
          Applies to all agents that don't have a per-agent model set.
        </p>
      </div>

      {/* Thinking / Effort */}
      <div className="p-3">
        <EffortSelectField
          currentEffort={currentEffort}
          effortDefault={effortDefault}
          effortValid={effortValid}
          htmlFor="global-agent-thinking-effort"
          inheritFallbackLabel={
            effortDefault !== null ? `Default (${effortDefault})` : undefined
          }
          inheritedEffort={bakedEffort ?? undefined}
          label="Default thinking / effort"
          onChange={(value) => {
            const nextEnvVars = { ...config.env_vars };
            if (value === "") {
              delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
            } else {
              nextEnvVars[BUZZ_AGENT_THINKING_EFFORT] = value;
            }
            onConfigChange({ ...config, env_vars: nextEnvVars });
          }}
          testId="global-agent-thinking-effort-select"
        />
        <p className="mt-1.5 text-xs text-muted-foreground">
          Default thinking/reasoning effort applied to all agents. Per-agent
          settings override this.
        </p>
      </div>

      {/* Env vars */}
      <div className="p-3">
        <EnvVarsEditor
          helperText="Injected into all agents as the lowest-priority layer. Per-agent values override these."
          hiddenKeys={apiKeyEnvVar ? [apiKeyEnvVar] : []}
          inheritedRows={bakedGenericRows}
          inheritedRowsLabel="build"
          label="Global environment variables"
          onChange={handleEnvVarsChange}
          requiredKeys={advancedRequiredEnvKeys}
          value={Object.fromEntries(
            Object.entries(config.env_vars).filter(
              ([k]) => k !== BUZZ_AGENT_THINKING_EFFORT,
            ),
          )}
        />
      </div>
    </SettingsOptionGroup>
  );
}
