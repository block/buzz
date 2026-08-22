/**
 * useAgentEditRuntimeState — runtime/model/provider computed state for the
 * merged agent edit surface.
 *
 * Extracted from AgentEditMergedDialog to keep that file within the 1000-line
 * desktop size gate. Contains all the React.useMemo/useEffect that derives
 * dropdown options, model discovery, provider gate, and credential state.
 */

import * as React from "react";

import { useAgentDialogDefaults } from "./useAgentDialogDefaults";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import { useRequiredCredentialState } from "./useRequiredCredentialState";
import {
  getBakedProviderInheritLabel,
  getBakedModelInheritLabel,
} from "./bakedEnvHelpers";
import { ADD_CUSTOM_HARNESS_OPTION } from "./addCustomHarness";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getDefaultPersonaRuntime,
  getPersonaProviderOptions,
  NO_RUNTIME_DROPDOWN_VALUE,
  buildPersonaRuntimeDropdownOptions,
  runtimeSupportsLlmProviderSelection,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
  type PersonaDropdownOption,
  getProviderApiKeyEnvVar,
  getDefaultLlmModelLabel,
} from "./agentConfigOptions";
import {
  MODEL_DISCOVERY_LOADING_VALUE,
  usePersonaModelDiscovery,
} from "./usePersonaModelDiscovery";
import {
  modelDropdownOptions as buildModelDropdownOptions,
  relayMeshModelPickerState,
} from "./relayMeshModelPicker";
import { resolveModelFieldStatusMessage } from "./agentConfigControls";
import {
  resolveInheritedRuntimeSubmission,
  hasMissingRequiredEnvKey,
} from "./personaRuntimeModel";
import { useRuntimeFileConfigQuery } from "../hooks";
import type { AcpRuntimeCatalogEntry, ManagedAgent } from "@/shared/api/types";
import type { AgentPersona } from "@/shared/api/types";

// ── Types ─────────────────────────────────────────────────────────────────────

/**
 * Definition-context advanced state for the D-section (parity with main's
 * `AgentDefinitionDialog` edit mode + `PersonaAdvancedFields`).
 *
 * Every field here is derived from the DEFINITION's runtime/provider/env
 * (`definitionRuntimeId`, the D-layer provider, and the D-env `envVars`) —
 * independent of the instance harness pin. In linked context the instance
 * overlay (I-section) keeps using the prospective-runtime state; the D-section
 * uses this bundle so its tuning knobs, env-key highlighting, and API-key field
 * reflect the definition, not the instance.
 *
 * Bundled into a single object so the parent threads one prop into the
 * D-section rather than a dozen — the merged dialog file sits against the
 * desktop line-size gate.
 */
export type DSectionAdvancedState = {
  /** Definition runtime id — drives the buzz-agent effort-tuning gate. */
  runtimeId: string;
  /** Catalog entry for the definition runtime — drives numeric descriptors and the parallelism cap hint. */
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  /** Effective definition provider (agent value → global fallback) — for tuning-field filtering and the API-key label. */
  effectiveProvider: string;
  /** Inherited env layer for tuning-field placeholders (global / template defaults). */
  inheritedEnvVars: Record<string, string>;
  /** Required non-secret env keys highlighted in the env editor (secret key excluded — it renders as the API-key field). */
  advancedRequiredEnvKeys: readonly string[];
  /** Required keys already satisfied by the runtime file layer (shown as info rows). */
  fileSatisfiedEnvKeys: readonly string[];
  /** Definition-level provider secret env var, or null when the provider has none. */
  topLevelSecretEnvVar: string | null;
  apiKeyValue: string;
  apiKeyIsInherited: boolean;
  apiKeyInheritedLabel: string;
  apiKeyIsRequired: boolean;
  /**
   * Any required credential key (secret or non-secret) is still unset for the
   * definition runtime/provider. Gates Save whenever the definition is editable,
   * mirroring main's `localModeSatisfied` credential arm.
   */
  requiredEnvKeyMissing: boolean;
  /**
   * A non-secret required env key (rendered as a generic Advanced row, not the
   * API-key field) is still missing. Drives the collapsed-Advanced "Required"
   * badge, matching main's `missingEnvKeys ∩ advancedRequiredEnvKeys`.
   */
  advancedRequiredEnvKeyMissing: boolean;
};

export type AgentEditRuntimeStateInputs = {
  open: boolean;
  showDef: boolean;
  showInst: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus: "loading" | "error" | "ready";
  selectedRuntimeId: string;
  definitionRuntimeId: string;
  model: string;
  provider: string;
  isCustomModelEditing: boolean;
  isCustomProviderEditing: boolean;
  envVars: Record<string, string>;
  instanceEnvVars: Record<string, string>;
  inheritHarness: boolean;
  inst: ManagedAgent | null;
  def: AgentPersona | null;
  linkedPersona: AgentPersona | null;
  bakedEnvKeys: string[] | undefined;
  originalAgentCommand: string;
  setModel: (v: string) => void;
  setIsCustomModelEditing: (v: boolean) => void;
};

export type AgentEditRuntimeStateResult = {
  sortedRuntimes: AcpRuntimeCatalogEntry[];
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  runtimeDropdownValue: string;
  defRuntimeDropdownValue: string;
  instanceRuntimeDropdownOptions: PersonaDropdownOption[];
  defRuntimeDropdownOptions: PersonaDropdownOption[];
  defBlankLabel: string;
  originalRuntimeSupportsProvider: boolean;
  prospectiveRuntimeId: string;
  prospectiveRuntime: AcpRuntimeCatalogEntry | undefined;
  llmProviderFieldVisible: boolean;
  inheritedEnvVars: Record<string, string>;
  inheritedSubmission: {
    model: string | null;
    provider: string | null;
    envVars: Record<string, string>;
  };
  globalConfig: { env_vars: Record<string, string> };
  inheritedModelDefault: import("./bakedEnvHelpers").InheritedDefault;
  inheritedProviderDefault: import("./bakedEnvHelpers").InheritedDefault;
  inheritedEnvVarsForAdvanced: Record<string, string>;
  requiredEnvKeys: string[];
  fileSatisfiedEnvKeys: string[];
  requiredEnvKeyMissing: boolean;
  effectiveProvider: string;
  providerDropdownOptions: PersonaDropdownOption[];
  providerSelectValue: string;
  modelDropdownOptions: PersonaDropdownOption[];
  modelSelectValue: string;
  showCustomModelInput: boolean;
  modelStatusMessage: string | null;
  providerApiKeyEnvVar: string | null;
  apiKeyValue: string;
  apiKeyIsInherited: boolean;
  apiKeyInheritedLabel: string;
  apiKeyIsRequired: boolean;
  topLevelSecretEnvVar: string | null;
  advancedRequiredEnvKeys: readonly string[];
  /** True while model discovery is in progress (drives model dropdown disabled state). */
  modelDiscoveryLoading: boolean;
  /**
   * True when the definition has no runtime but has a saved model/provider,
   * making the provider/model picker editable without a runtime selection.
   * Matches AgentDefinitionDialog's blankRuntimeModelProviderEditable contract.
   */
  blankRuntimeModelProviderEditable: boolean;
  /**
   * Definition-context advanced state consumed by the D-section (tuning knobs,
   * env-key highlighting, provider API-key field). Derived from the definition
   * runtime/provider/env so the D-section matches main's PersonaAdvancedFields.
   */
  dAdvanced: DSectionAdvancedState;
};

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useAgentEditRuntimeState({
  open,
  showDef,
  showInst,
  runtimes,
  runtimeCatalogStatus,
  selectedRuntimeId,
  definitionRuntimeId,
  model,
  provider,
  isCustomModelEditing,
  isCustomProviderEditing,
  envVars,
  instanceEnvVars,
  inheritHarness,
  inst,
  def,
  linkedPersona,
  bakedEnvKeys,
  originalAgentCommand,
  setModel,
  setIsCustomModelEditing,
}: AgentEditRuntimeStateInputs): AgentEditRuntimeStateResult {
  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(runtimes),
    [runtimes],
  );
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const runtimeDropdownValue = selectedRuntimeId || NO_RUNTIME_DROPDOWN_VALUE;
  const defRuntimeDropdownValue =
    definitionRuntimeId || NO_RUNTIME_DROPDOWN_VALUE;

  const instanceRuntimeDropdownOptions: PersonaDropdownOption[] =
    React.useMemo(() => {
      const options: PersonaDropdownOption[] = [
        ...sortedRuntimes.map((r) => ({
          label: formatRuntimeOptionLabel(r),
          value: r.id,
        })),
        { label: "Custom command", value: "custom" },
      ];
      if (
        selectedRuntimeId &&
        selectedRuntimeId !== "custom" &&
        !options.some((o) => o.value === selectedRuntimeId)
      ) {
        options.push({
          label: `${selectedRuntimeId} (current)`,
          value: selectedRuntimeId,
        });
      }
      options.push(ADD_CUSTOM_HARNESS_OPTION);
      return options;
    }, [sortedRuntimes, selectedRuntimeId]);

  const {
    blankRuntimeOptionLabel: defBlankLabel,
    runtimeDropdownOptions: defRuntimeDropdownOptions,
  } = React.useMemo(() => {
    const result = buildPersonaRuntimeDropdownOptions({
      defaultRuntimeId: getDefaultPersonaRuntime(runtimes)?.id,
      isCreateMode: false,
      runtime: definitionRuntimeId === "custom" ? "" : definitionRuntimeId,
      runtimes,
      runtimesLoading: runtimeCatalogStatus === "loading",
    });
    result.runtimeDropdownOptions.push(ADD_CUSTOM_HARNESS_OPTION);
    return result;
  }, [runtimes, runtimeCatalogStatus, definitionRuntimeId]);

  const originalRuntimeSupportsProvider = React.useMemo(() => {
    if (!inst) return false;
    const originalCommand = originalAgentCommand.trim();
    const matched =
      runtimes.find((r) => r.command?.trim() === originalCommand) ??
      runtimes.find((r) => r.id === originalCommand);
    return runtimeSupportsLlmProviderSelection(matched?.id ?? "");
  }, [runtimes, originalAgentCommand, inst]);

  const prospectiveRuntimeId = React.useMemo(() => {
    if (!inst) return selectedRuntimeId || "";
    if (!inheritHarness) return selectedRuntime?.id ?? selectedRuntimeId;
    const personaRuntimeId = linkedPersona?.runtime?.trim();
    if (personaRuntimeId) {
      return (
        runtimes.find((r) => r.id === personaRuntimeId)?.id ?? personaRuntimeId
      );
    }
    return (
      runtimes.find((r) => r.command?.trim() === inst.agentCommand.trim())
        ?.id ??
      runtimes.find((r) => r.id === inst.agentCommand.trim())?.id ??
      getDefaultPersonaRuntime(runtimes)?.id ??
      ""
    );
  }, [
    inheritHarness,
    linkedPersona?.runtime,
    runtimes,
    inst?.agentCommand,
    selectedRuntime?.id,
    selectedRuntimeId,
    inst,
  ]);

  // The active provider/model layer, chosen ONCE for every provider/model
  // derivation below (visibility, provider options, model discovery, scope
  // clearing): the DEFINITION runtime whenever a definition is present (linked
  // or definition-only — its picker is the one rendered), and the instance
  // harness pin only for unlinked instance-only agents. Main splits this across
  // two dialogs; the merged hook selects the layer here so the D-section picker
  // follows the definition and the I-section follows the instance pin, instead
  // of both multiplexing on `showInst` per call site.
  const activeRuntimeId = showDef ? definitionRuntimeId : prospectiveRuntimeId;

  const llmProviderFieldVisible = React.useMemo(() => {
    if (runtimeSupportsLlmProviderSelection(activeRuntimeId)) return true;

    // blankRuntimeModelProviderEditable: for an edit context (not create), when the
    // definition has no runtime configured but already has a saved model or provider,
    // expose the provider/model picker so the user can edit the existing values.
    // This matches AgentDefinitionDialog's `initialModelProviderEditableWithoutRuntime`
    // contract introduced in commit 87dc4dccba — restoring the capability that was
    // accidentally dropped when test-11 was rewritten.
    if (showDef && activeRuntimeId === "custom") {
      // The outer guard already means this is a runtime-less definition, so
      // expose the picker as soon as there is a saved model or provider to edit.
      const hasSavedModelOrProvider =
        (linkedPersona?.model ?? def?.model ?? "").trim().length > 0 ||
        (linkedPersona?.provider ?? def?.provider ?? "").trim().length > 0;
      if (hasSavedModelOrProvider) return true;
    }

    return false;
  }, [activeRuntimeId, showDef, linkedPersona, def]);
  const prospectiveRuntime = runtimes.find(
    (r) => r.id === prospectiveRuntimeId,
  );
  // The catalog entry for the active provider/model layer (definition when a
  // definition is present, instance harness pin otherwise). Every provider/model
  // derivation below reads this so the D-section follows the definition runtime
  // and the I-section follows the instance pin — never cross-contaminated.
  const activeRuntime = runtimes.find((r) => r.id === activeRuntimeId);

  const inheritedEnvVars = linkedPersona?.envVars ?? {};
  const inheritedSubmission = React.useMemo(() => {
    if (!inst)
      return { provider: provider || null, model: model || null, envVars };
    return resolveInheritedRuntimeSubmission({
      inheritHarness,
      agentWasHarnessPinned: inst.agentCommandOverride != null,
      provider,
      personaProvider: linkedPersona?.provider ?? "",
      model,
      personaModel: linkedPersona?.model ?? null,
      envVars: showDef ? envVars : instanceEnvVars,
      personaEnvVars: inheritedEnvVars,
    });
  }, [
    inheritHarness,
    inst,
    linkedPersona,
    provider,
    model,
    envVars,
    instanceEnvVars,
    inheritedEnvVars,
    showDef,
  ]);

  const {
    globalConfig,
    inheritedDefaults: {
      provider: inheritedProviderDefault,
      model: inheritedModelDefault,
    },
    inheritedEnvVars: inheritedEnvVarsForAdvanced,
  } = useAgentDialogDefaults({ inheritedEnvVars, open });

  const { requiredEnvKeys, fileSatisfiedEnvKeys, requiredEnvKeyMissing } =
    useRequiredCredentialState({
      open: open && showInst,
      prospectiveRuntimeId,
      provider: inheritedSubmission.provider ?? "",
      globalProvider: inheritedProviderDefault.value,
      envVars: inheritedSubmission.envVars,
      globalEnvVars: globalConfig.env_vars,
      personaEnvVars: inheritHarness ? inheritedEnvVars : undefined,
    });

  useRuntimeFileConfigQuery(
    showInst ? prospectiveRuntimeId : selectedRuntimeId,
    { enabled: open },
  );

  // ── Definition-context advanced state (D-section parity) ───────────────────
  // The instance-side credential/api-key state above is gated on `showInst`, so
  // in definition-only context it starves the D-section. Compute the definition
  // layer independently from `definitionRuntimeId` + the D provider + the D env,
  // mirroring main's AgentDefinitionDialog. Runs in every context (the query is
  // `enabled: open`); the D-section only consumes it when `showDef`.
  const defSelectedRuntime = runtimes.find((r) => r.id === definitionRuntimeId);
  const defEffectiveProvider =
    provider.trim() || inheritedProviderDefault.value;
  // D-only inherited env layer: global + build defaults with NO persona env.
  // Main's AgentDefinitionDialog calls useAgentDialogDefaults({ open }) with no
  // persona env, so a definition tuning value that is cleared falls back to the
  // global/build default — never to the definition's own just-deleted value.
  // The instance overlay keeps the persona-inclusive `inheritedEnvVarsForAdvanced`.
  const { inheritedEnvVars: dInheritedEnvVars } = useAgentDialogDefaults({
    open,
  });
  const defRequired = useRequiredCredentialState({
    open,
    prospectiveRuntimeId: definitionRuntimeId,
    provider: defEffectiveProvider,
    globalProvider: inheritedProviderDefault.value,
    envVars,
    globalEnvVars: globalConfig.env_vars,
  });
  const defApiKeyState = useProviderApiKeyFieldState({
    bakedEnvKeys,
    effectiveEnvVars: envVars,
    envVars,
    fileSatisfiedEnvKeys: defRequired.fileSatisfiedEnvKeys,
    globalEnvVars: globalConfig.env_vars,
    provider: defEffectiveProvider,
    requiredEnvKeys: defRequired.requiredEnvKeys,
  });
  const dAdvanced: DSectionAdvancedState = {
    runtimeId: definitionRuntimeId,
    selectedRuntime: defSelectedRuntime,
    effectiveProvider: defEffectiveProvider,
    inheritedEnvVars: dInheritedEnvVars,
    advancedRequiredEnvKeys: defApiKeyState.advancedRequiredEnvKeys,
    fileSatisfiedEnvKeys: defRequired.fileSatisfiedEnvKeys,
    topLevelSecretEnvVar: defApiKeyState.secretEnvVar,
    apiKeyValue: defApiKeyState.value,
    apiKeyIsInherited: defApiKeyState.isInherited,
    apiKeyInheritedLabel: defApiKeyState.inheritedLabel,
    apiKeyIsRequired: defApiKeyState.isRequired,
    requiredEnvKeyMissing: defRequired.requiredEnvKeyMissing,
    advancedRequiredEnvKeyMissing: hasMissingRequiredEnvKey(
      defApiKeyState.advancedRequiredEnvKeys,
      envVars,
    ),
  };

  const effectiveProvider =
    (inheritedSubmission.provider ?? "").trim() ||
    inheritedProviderDefault.value;
  const providerForDiscovery = llmProviderFieldVisible ? effectiveProvider : "";
  const envVarsForDiscovery = React.useMemo(
    () => ({ ...globalConfig.env_vars, ...inheritedSubmission.envVars }),
    [globalConfig.env_vars, inheritedSubmission.envVars],
  );

  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars: envVarsForDiscovery,
    isCustomProviderEditing,
    modelFieldVisible: true,
    open,
    provider: providerForDiscovery,
    selectedRuntime: activeRuntime,
  });

  const hideProviderIds = React.useMemo(
    () =>
      (bakedEnvKeys ?? []).includes("BUZZ_AGENT_PROVIDER")
        ? BLOCK_BUILD_HIDDEN_PROVIDER_IDS
        : new Set<string>(),
    [bakedEnvKeys],
  );

  const providerOptions = getPersonaProviderOptions(
    provider.trim(),
    activeRuntimeId,
    inheritedProviderDefault.source === "global"
      ? inheritedProviderDefault.value
      : "",
    hideProviderIds,
  );
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : provider.trim() || AUTO_PROVIDER_DROPDOWN_VALUE;
  const providerDropdownOptions: PersonaDropdownOption[] = [
    ...providerOptions.map((opt) => ({
      label:
        opt.id === "" && inheritedProviderDefault.source === "build"
          ? getBakedProviderInheritLabel(
              inheritedProviderDefault.value,
              providerOptions,
            )
          : opt.label,
      value: opt.id || AUTO_PROVIDER_DROPDOWN_VALUE,
    })),
    { label: "Custom provider...", value: CUSTOM_PROVIDER_DROPDOWN_VALUE },
  ];

  const {
    isRelayMesh,
    options: effectiveModelOptions,
    selectValue: modelSelectValue,
    showCustomInput: showCustomModelInput,
  } = relayMeshModelPickerState({
    discoveredOptions: discoveredModelOptions,
    fallbackOptions: [
      { id: "", label: resolveInheritedModelLabel(inheritedModelDefault) },
    ],
    isCustomEditing: isCustomModelEditing,
    model,
    provider: providerForDiscovery,
  });

  const modelDropdownOptions = buildModelDropdownOptions({
    allowCustom: !isRelayMesh,
    globalModel: isRelayMesh ? undefined : inheritedModelDefault.value,
    globalModelLabel: isRelayMesh
      ? undefined
      : resolveInheritedModelLabel(inheritedModelDefault),
    loading: modelDiscoveryLoading && discoveredModelOptions === null,
    loadingValue: MODEL_DISCOVERY_LOADING_VALUE,
    options: effectiveModelOptions,
  });

  const modelStatusMessage = resolveModelFieldStatusMessage({
    discoveredModelOptions,
    loading: modelDiscoveryLoading,
    status: modelDiscoveryStatus,
  });

  const providerApiKeyEnvVar = getProviderApiKeyEnvVar(effectiveProvider);
  const personaSatisfied =
    providerApiKeyEnvVar != null &&
    !(providerApiKeyEnvVar in (showInst ? instanceEnvVars : envVars)) &&
    (inheritedEnvVars[providerApiKeyEnvVar] ?? "").length > 0;
  const apiKeyFieldState = useProviderApiKeyFieldState({
    bakedEnvKeys,
    effectiveEnvVars: inheritedSubmission.envVars,
    envVars: showInst ? instanceEnvVars : envVars,
    fileSatisfiedEnvKeys,
    globalEnvVars: globalConfig.env_vars,
    personaSatisfied,
    provider: effectiveProvider,
    requiredEnvKeys,
  });
  const {
    advancedRequiredEnvKeys,
    inheritedLabel: apiKeyInheritedLabel,
    isInherited: apiKeyIsInherited,
    isRequired: apiKeyIsRequired,
    secretEnvVar: topLevelSecretEnvVar,
    value: apiKeyValue,
  } = apiKeyFieldState;

  // Clear model when provider scope changes
  React.useEffect(() => {
    if (
      !open ||
      isCustomModelEditing ||
      !shouldClearKnownModelForSelectionScope({
        model,
        provider: providerForDiscovery,
        runtime: activeRuntimeId,
      })
    )
      return;
    setModel("");
    setIsCustomModelEditing(false);
  }, [
    isCustomModelEditing,
    model,
    open,
    providerForDiscovery,
    activeRuntimeId,
    setModel,
    setIsCustomModelEditing,
  ]);

  return {
    sortedRuntimes,
    selectedRuntime,
    runtimeDropdownValue,
    defRuntimeDropdownValue,
    instanceRuntimeDropdownOptions,
    defRuntimeDropdownOptions,
    defBlankLabel,
    originalRuntimeSupportsProvider,
    prospectiveRuntimeId,
    prospectiveRuntime,
    llmProviderFieldVisible,
    inheritedEnvVars,
    inheritedSubmission,
    globalConfig,
    inheritedModelDefault,
    inheritedProviderDefault,
    inheritedEnvVarsForAdvanced,
    requiredEnvKeys,
    fileSatisfiedEnvKeys,
    requiredEnvKeyMissing,
    effectiveProvider,
    providerDropdownOptions,
    providerSelectValue,
    modelDropdownOptions,
    modelSelectValue,
    showCustomModelInput,
    modelStatusMessage,
    providerApiKeyEnvVar,
    apiKeyValue,
    apiKeyIsInherited,
    apiKeyInheritedLabel,
    apiKeyIsRequired,
    topLevelSecretEnvVar,
    advancedRequiredEnvKeys,
    modelDiscoveryLoading,
    blankRuntimeModelProviderEditable:
      showDef &&
      definitionRuntimeId === "custom" &&
      ((linkedPersona?.model ?? def?.model ?? "").trim().length > 0 ||
        (linkedPersona?.provider ?? def?.provider ?? "").trim().length > 0),
    dAdvanced,
  };
}

function resolveInheritedModelLabel(
  inherited: import("./bakedEnvHelpers").InheritedDefault,
): string {
  const model = inherited.value;
  if (!model) return "Default model";
  if (inherited.source === "build") return getBakedModelInheritLabel(model);
  if (inherited.source === "global") return getDefaultLlmModelLabel(model);
  return `Default (${model})`;
}
