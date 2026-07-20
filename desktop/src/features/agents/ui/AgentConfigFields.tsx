/**
 * Controlled field group for global agent config (provider, model, effort, env vars).
 *
 * Used by AgentDefaultsSettingsCard (settings panel) and AgentDefaultsSection
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
  deriveAgentConfigFieldModel,
  getRenderableEffortField,
  hasRenderableAgentConfigField,
} from "@/features/agents/lib/agentConfigCore";
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
  runtimeSupportsLlmProviderSelection,
} from "@/features/agents/ui/agentConfigOptions";
import {
  AgentDropdownSelect,
  AgentModelField,
} from "@/features/agents/ui/agentConfigControls";
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

// Canonical behaviors (PR 2 flag cleanup). These were per-surface props;
// onboarding's values won every call and are now the only behavior:
// - auto-select a valid model when the provider changes
// - keep the model select usable during discovery
// - preserve credential env vars across provider switches (the abandoned
//   provider's key stays in env_vars — visible/deletable under Advanced —
//   so flipping back never loses a typed key; spawned agents may therefore
//   see credentials for providers they don't use)
// - require a provider before model/effort are editable (no saveable
//   invalid state — design principle #4). Note: legacy configs saved with
//   a model but no provider are cleared by the pre-existing orphan-model
//   effect on next edit — deliberate data healing, documented in PR.
const autoSelectModelOnProviderChange = true;
const disableModelSelectDuringDiscovery = false;
const preserveCredentialEnvVarsOnProviderChange = true;
const requireProviderForModelAndEffort = true;

/** The canonical behavior contract, exported for the contract test. */
export const CANONICAL_CONFIG_BEHAVIORS = {
  autoSelectModelOnProviderChange,
  disableModelSelectDuringDiscovery,
  preserveCredentialEnvVarsOnProviderChange,
  requireProviderForModelAndEffort,
} as const;

/**
 * Disclosure preset → the eight visibility decisions it owns. Effort is
 * shown in both presets (onboarding never hid it; the old prop existed but
 * was never flipped). Exported for the contract test.
 */
export function resolveDisclosure(disclosure: "full" | "onboarding-essential") {
  const full = disclosure === "full";
  return {
    showAdvancedFields: full,
    showCustomModelOption: full,
    showCustomProviderOption: full,
    showDescriptions: full,
    showEffortField: true,
    showProviderPlaceholderOption: full,
    showRequiredIndicators: full,
    showUnavailableEffortOptions: full,
  } as const;
}

export type AgentConfigFieldsProps = {
  bakedEnv: BakedEnvEntry[];
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  config: GlobalAgentConfig;
  isCustomModelEditing: boolean;
  isCustomProvider: boolean;
  onConfigChange: (next: GlobalAgentConfig) => void;
  onCustomModelEditingChange: (value: boolean) => void;
  onIsCustomProviderChange: (value: boolean) => void;
  onValidityChange?: (valid: boolean) => void;
  placeholderClassName?: string;
  selectClassName?: string;
  /**
   * Which disclosure preset to render (PR 2 flag cleanup — replaces eight
   * independent show* booleans):
   * - "full" (default): the evergreen stance — every field, escape hatch
   *   (custom model/provider), description, required indicator, and
   *   unavailable option is visible. Settings, defaults modal, dialogs.
   * - "onboarding-essential": onboarding page 4's first-run stance — only
   *   valid forward choices. No advanced section, no custom escape hatches,
   *   no descriptions (the page copy does that job), no un-choosing via
   *   placeholder options, no greyed-out effort levels.
   * If a second surface wants the trimmed view, rename this value to plain
   * "essential" — and have the conversation about whether it should really
   * match onboarding.
   */
  disclosure?: "full" | "onboarding-essential";
  unstyled?: boolean;
  useCustomSelect?: boolean;
  useChevronSelectIcon?: boolean;
};

export function AgentConfigFields({
  bakedEnv,
  selectedRuntime,
  config,
  isCustomModelEditing,
  isCustomProvider,
  onConfigChange,
  onCustomModelEditingChange,
  onIsCustomProviderChange,
  onValidityChange,
  placeholderClassName,
  selectClassName,
  disclosure = "full",
  unstyled = false,
  useCustomSelect = false,
  useChevronSelectIcon = false,
}: AgentConfigFieldsProps) {
  const {
    showAdvancedFields,
    showCustomModelOption,
    showCustomProviderOption,
    showDescriptions,
    showEffortField,
    showProviderPlaceholderOption,
    showRequiredIndicators,
    showUnavailableEffortOptions,
  } = resolveDisclosure(disclosure);

  const fieldModel = React.useMemo(
    () =>
      deriveAgentConfigFieldModel({
        config,
        runtime: selectedRuntime,
        scope: disclosure === "onboarding-essential" ? "onboarding" : "global",
      }),
    [config, disclosure, selectedRuntime],
  );
  const effortField = getRenderableEffortField(fieldModel);
  const effortPersistenceKey =
    effortField?.currentPersistence.kind === "envVar"
      ? effortField.currentPersistence.key
      : null;
  const bakedProvider = React.useMemo(
    () => bakedEnv.find((e) => e.key === "BUZZ_AGENT_PROVIDER")?.value ?? null,
    [bakedEnv],
  );
  const selectedRuntimeId = selectedRuntime?.id ?? "";
  const providerFieldVisible = hasRenderableAgentConfigField(
    fieldModel,
    "provider",
  );
  const effectiveProvider = providerFieldVisible
    ? config.provider?.trim() || bakedProvider || ""
    : "";
  const fallbackModel = React.useMemo(
    () => getGlobalModelFallback(bakedEnv, effectiveProvider, config.env_vars),
    [bakedEnv, config.env_vars, effectiveProvider],
  );
  const modelField = fieldModel.fields.find(
    (field) => field.kind === "model" && field.render === "control",
  );
  // CLI-login harnesses apply this setting through ACP rather than an env var
  // and provide their own default when no model override is persisted.
  const modelIsOptional = modelField?.targetApplication.kind === "acpNative";
  const modelIsValid =
    modelIsOptional ||
    (config.model?.trim().length ?? 0) > 0 ||
    fallbackModel !== null;
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

  const providerValue = providerFieldVisible ? (config.provider ?? "") : "";
  const providerForDiscovery =
    providerFieldVisible && !isCustomProvider
      ? providerValue || bakedProvider || ""
      : "";
  const dependentFieldsDisabled =
    providerFieldVisible &&
    requireProviderForModelAndEffort &&
    providerForDiscovery.trim().length === 0;
  const credentialProvider =
    providerFieldVisible && !isCustomProvider ? effectiveProvider : "";
  const credentialRuntimeId = runtimeSupportsLlmProviderSelection(
    selectedRuntimeId,
  )
    ? selectedRuntimeId
    : "buzz-agent";
  const requiredEnvKeys = requiredCredentialEnvKeys(
    credentialRuntimeId,
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

  // Mount-time healing policy: onboarding page 4 edits the root config during
  // first-run (no higher layers to inherit from), so acting on open is safe
  // and intentional there — it heals stale state and picks a valid model.
  // Evergreen surfaces (Settings, dialogs) edit saved data that may pair with
  // higher layers (see PR #2148 review thread), so they only act after the
  // user explicitly edits the provider in this session.
  const healOnMount =
    fieldModel.dependentValuePolicy.onCatalogMismatch === "onboardingCleanup";
  const userEditedProviderRef = React.useRef(false);
  // Env vars live under a collapsed Advanced section (matching the create
  // flow). Auto-open when a required key is missing so the field the user
  // must fill is never hidden behind the toggle.
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
  const requiredAdvancedKeyMissing = advancedRequiredEnvKeys.some(
    (key) => !(config.env_vars[key] ?? "").trim(),
  );
  React.useEffect(() => {
    if (requiredAdvancedKeyMissing) setAdvancedOpen(true);
  }, [requiredAdvancedKeyMissing]);
  // Read inside effects via ref so biome's exhaustive-deps stays honest:
  // refs are stable, and healOnMount is captured at declaration.
  const mayMutateDependentFieldsRef = React.useRef(false);
  mayMutateDependentFieldsRef.current =
    healOnMount || userEditedProviderRef.current;

  const autoSelectedModelScopeRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!autoSelectModelOnProviderChange) return;
    if (!mayMutateDependentFieldsRef.current) return;
    const trimmedProvider = providerForDiscovery.trim();
    if (trimmedProvider.length === 0 || isCustomProvider) {
      autoSelectedModelScopeRef.current = null;
      return;
    }
    if ((config.model ?? "").trim().length > 0) return;
    if (modelDiscoveryLoading || discoveredModelOptions === null) return;
    const selectionScope = `${selectedRuntimeId}:${trimmedProvider}`;
    if (autoSelectedModelScopeRef.current === selectionScope) return;

    const firstModel = discoveredModelOptions.find(
      (option) => option.id.trim().length > 0,
    );
    if (!firstModel) return;

    autoSelectedModelScopeRef.current = selectionScope;
    onCustomModelEditingChange(false);
    onConfigChange({ ...config, model: firstModel.id });
  }, [
    config,
    discoveredModelOptions,
    isCustomProvider,
    modelDiscoveryLoading,
    onConfigChange,
    onCustomModelEditingChange,
    providerForDiscovery,
    selectedRuntimeId,
  ]);

  const currentEffortForAutoClear = effortPersistenceKey
    ? (config.env_vars[effortPersistenceKey] ?? "")
    : "";

  // When the selected harness changes outside this component (Back → setup
  // page → choose a different harness → Next), the saved model can belong to
  // the old harness. In onboarding, heal that stale value as soon as the new
  // harness catalog proves it is unsupported; otherwise a Codex id like
  // `gpt-5.5[low]` appears as a Claude Code custom model.
  React.useEffect(() => {
    if (!healOnMount) return;
    const currentModel = (config.model ?? "").trim();
    if (currentModel.length === 0) return;
    if (modelDiscoveryLoading || discoveredModelOptions === null) return;
    if (
      discoveredModelOptions.some((option) => option.id.trim() === currentModel)
    ) {
      return;
    }

    const nextEnvVars = { ...config.env_vars };
    if (effortPersistenceKey) delete nextEnvVars[effortPersistenceKey];
    onCustomModelEditingChange(false);
    onConfigChange({ ...config, env_vars: nextEnvVars, model: null });
  }, [
    config,
    discoveredModelOptions,
    modelDiscoveryLoading,
    onConfigChange,
    onCustomModelEditingChange,
    healOnMount,
    effortPersistenceKey,
  ]);

  // Orphan-model clearing follows the mount-time healing policy above: the
  // backend resolves provider and model independently across layers
  // (agent → definition → global), so a saved global model WITHOUT a global
  // provider can be a deliberate, working pattern (provider supplied by a
  // higher layer). Clearing it on page-open in evergreen surfaces silently
  // breaks that agent on its next restart — see PR #2148 review thread.
  // Onboarding heals on open by design (discriminating spec: "gates stale
  // saved model and effort until provider selection").
  React.useEffect(() => {
    if (!mayMutateDependentFieldsRef.current) return;
    if (!dependentFieldsDisabled) return;
    if (
      (config.model ?? "").trim().length === 0 &&
      currentEffortForAutoClear.length === 0
    ) {
      return;
    }

    const nextEnvVars = { ...config.env_vars };
    if (effortPersistenceKey) delete nextEnvVars[effortPersistenceKey];
    onCustomModelEditingChange(false);
    onConfigChange({ ...config, env_vars: nextEnvVars, model: null });
  }, [
    config,
    currentEffortForAutoClear,
    dependentFieldsDisabled,
    onConfigChange,
    onCustomModelEditingChange,
    effortPersistenceKey,
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
      if (effortPersistenceKey) delete nextEnvVars[effortPersistenceKey];
      onConfigChange({ ...config, env_vars: nextEnvVars });
    },
  });

  function handleProviderChange(value: string) {
    userEditedProviderRef.current = true;
    const previousApiKey = getProviderApiKeyEnvVar(effectiveProvider);
    if (value === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
      const nextEnvVars = { ...config.env_vars };
      if (!preserveCredentialEnvVarsOnProviderChange && previousApiKey) {
        delete nextEnvVars[previousApiKey];
      }
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
    if (
      !preserveCredentialEnvVarsOnProviderChange &&
      previousApiKey &&
      previousApiKey !== nextApiKey
    ) {
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
    const effort = effortPersistenceKey
      ? config.env_vars[effortPersistenceKey]
      : undefined;
    const merged = { ...next };
    if (effortPersistenceKey && effort !== undefined) {
      merged[effortPersistenceKey] = effort;
    }
    onConfigChange({ ...config, env_vars: merged });
  }

  // On internal Block builds, BUZZ_AGENT_PROVIDER is baked in and a boot
  // migration rewrites v1→v2. Hide the legacy v1 option so it is not offered
  // for new selections; OSS builds show it.
  const hideProviderIds = React.useMemo(() => {
    const hidden = new Set<string>();
    if (bakedEnvKeys.includes("BUZZ_AGENT_PROVIDER")) {
      for (const providerId of BLOCK_BUILD_HIDDEN_PROVIDER_IDS) {
        hidden.add(providerId);
      }
    }
    if (selectedRuntimeId !== "buzz-agent") {
      hidden.add("relay-mesh");
    }
    return hidden;
  }, [bakedEnvKeys, selectedRuntimeId]);
  const providerOptions = getPersonaProviderOptions(
    providerValue,
    credentialRuntimeId,
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

  const implicitEffortProvider =
    selectedRuntimeId === "claude"
      ? "anthropic"
      : selectedRuntimeId === "codex"
        ? "openai"
        : "";
  const effortProvider = providerFieldVisible
    ? (config.provider ?? "")
    : implicitEffortProvider;
  const { validValues: effortValid, defaultValue: effortDefault } =
    getProviderEffortConfig(effortProvider, config.model ?? "");
  const currentEffort = effortPersistenceKey
    ? (config.env_vars[effortPersistenceKey] ?? "")
    : "";
  const effortFieldVisible = showEffortField && effortField !== undefined;

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
      placeholderClassName={placeholderClassName}
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
      {providerFieldVisible ? (
        <div className={fieldClassName}>
          <label
            className={cn("text-sm font-medium", fieldLabelClassName)}
            htmlFor="global-agent-provider"
          >
            Provider
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
      ) : null}

      {providerFieldVisible && apiKeyEnvVar ? (
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
            !modelIsOptional &&
            fallbackModel === null &&
            !dependentFieldsDisabled
          }
          keepSelectedModelValueLabel
          model={dependentFieldsDisabled ? "" : (config.model ?? "")}
          modelDiscoveryLoading={
            dependentFieldsDisabled ? false : modelDiscoveryLoading
          }
          modelDiscoveryStatus={
            dependentFieldsDisabled ? null : modelDiscoveryStatus
          }
          onIsCustomModelEditingChange={onCustomModelEditingChange}
          onModelChange={handleModelChange}
          placeholderClassName={placeholderClassName}
          placeholder="Select a model"
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
      {effortFieldVisible ? (
        <div className={blockClassName}>
          <EffortSelectField
            currentEffort={dependentFieldsDisabled ? "" : currentEffort}
            disabled={dependentFieldsDisabled}
            emptyOptionLabel={
              // Semantic, not copy: onboarding-essential hides inheritance
              // concepts (first-run users pick, they don't inherit), so the
              // zero option is a plain placeholder. Full disclosure leaves
              // this unset so EffortSelectField computes the inherit/default
              // label ("Default (medium)", "Inherit (high)", …).
              disclosure === "onboarding-essential"
                ? "Select effort level"
                : undefined
            }
            effortDefault={effortDefault}
            effortValid={effortValid}
            fieldClassName={unstyled ? fieldClassName : undefined}
            htmlFor="global-agent-thinking-effort"
            inheritFallbackLabel={
              effortDefault !== null ? `Default (${effortDefault})` : undefined
            }
            inheritedEffort={bakedEffort ?? undefined}
            label="Effort"
            labelClassName={fieldLabelClassName}
            onChange={(value) => {
              const nextEnvVars = { ...config.env_vars };
              if (value === "") {
                if (effortPersistenceKey)
                  delete nextEnvVars[effortPersistenceKey];
              } else {
                if (effortPersistenceKey)
                  nextEnvVars[effortPersistenceKey] = value;
              }
              onConfigChange({ ...config, env_vars: nextEnvVars });
            }}
            placeholderClassName={placeholderClassName}
            selectClassName={selectClassName}
            showUnavailableOptions={showUnavailableEffortOptions}
            testId="global-agent-thinking-effort-select"
            useCustomSelect={useCustomSelect}
          />
        </div>
      ) : null}

      {showAdvancedFields ? (
        <div className={cn(blockClassName, "space-y-3")}>
          <button
            aria-expanded={advancedOpen}
            className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="global-agent-advanced-toggle"
            onClick={() => setAdvancedOpen((current) => !current)}
            type="button"
          >
            <span>Advanced</span>
            <ChevronDown
              className={cn(
                "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
                advancedOpen && "rotate-180",
              )}
            />
          </button>
          {advancedOpen ? (
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
          ) : null}
        </div>
      ) : null}
    </>
  );

  if (unstyled) {
    return <div className="space-y-7">{content}</div>;
  }

  return <SettingsOptionGroup>{content}</SettingsOptionGroup>;
}
