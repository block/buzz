/**
 * AgentEditMergedDialog — single merged edit surface for R1–R7.
 *
 * instance-with-definition → D + I + L sections on one form
 * instance-only            → I + L sections
 * definition-only          → D sections only
 *
 * Submit: emitAgentFormDiff → runAgentSaveCoordinator (observed-state settlement).
 * Submit logic extracted to useAgentEditMergedSubmit; section JSX to
 * AgentEditMergedDialogDSection / AgentEditMergedDialogInstanceSection.
 */

import * as React from "react";

import {
  useAcpRuntimesQuery,
  useBakedBuildEnvKeysQuery,
  usePersonasQuery,
  useStartManagedAgentMutation,
  useUpdateManagedAgentMutation,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { useUpdatePersonaAndPublishMutation } from "@/features/agents/lib/usePersonaCatalogRelay";
import type { ManagedAgent, RespondToMode } from "@/shared/api/types";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

import {
  seedAgentFormModel,
  hasDefinitionContext,
  hasInstanceContext,
  isDefinitionReadOnly,
  editContextDefinition,
  editContextInstance,
  type AgentEditContext,
} from "./agentFormModel";
import { useAgentEditMergedSubmit } from "./useAgentEditMergedSubmit";
import { AgentEditMergedDSection } from "./AgentEditMergedDialogDSection";
import { AgentEditMergedInstanceSection } from "./AgentEditMergedDialogInstanceSection";
import {
  ADD_CUSTOM_HARNESS_OPTION,
  runtimeDropdownAction,
  usePendingHarnessSelection,
} from "./addCustomHarness";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getDefaultPersonaRuntime,
  getPersonaProviderOptions,
  isMissingRequiredDropdownField,
  NO_RUNTIME_DROPDOWN_VALUE,
  buildPersonaRuntimeDropdownOptions,
  runtimeSupportsLlmProviderSelection,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import {
  selectionOnModelDropdownChange,
  selectionOnProviderDropdownChange,
  selectionOnRuntimeChange,
  type RuntimeModelProviderSelection,
} from "./runtimeModelProviderSelection";
import {
  MODEL_DISCOVERY_LOADING_VALUE,
  usePersonaModelDiscovery,
} from "./usePersonaModelDiscovery";
import {
  modelDropdownOptions as buildModelDropdownOptions,
  relayMeshModelPickerState,
} from "./relayMeshModelPicker";
import { AgentCreationPreview } from "./AgentCreationPreview";
import { useAgentDialogDefaults } from "./useAgentDialogDefaults";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import { useRequiredCredentialState } from "./useRequiredCredentialState";
import { getBakedProviderInheritLabel } from "./bakedEnvHelpers";
import { getProviderApiKeyEnvVar } from "./agentConfigOptions";
import { resolveModelFieldStatusMessage } from "./agentConfigControls";
import { useAgentAccessOwnerOnlyQuery } from "@/features/agents/useAgentAccessOwnerOnly";
import {
  resolveInheritedRuntimeSubmission,
  isEditAgentProviderSaveValid,
} from "./personaRuntimeModel";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { formatPersonaNamePoolText } from "./personaDialogState";
import { useRuntimeFileConfigQuery } from "../hooks";

// ── Types / Component ────────────────────────────────────────────────────────

export type AgentEditMergedDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  ctx: AgentEditContext;
  /** Optional field to focus when the dialog opens from a card deep-link. */
  initialFocus?: EditAgentFocusTarget;
  onUpdated?: (agent: ManagedAgent) => void;
  /**
   * Optional pre-save validator (R6 origin permission check).
   * Return a non-null string to abort with an error toast; return null to proceed.
   */
  onValidate?: () => string | null;
  /**
   * Optional initial-values override for definition fields (R6 review mode).
   */
  initialValueOverrides?: Partial<{
    displayName: string;
    systemPrompt: string;
    runtime: string | undefined;
    provider: string | undefined;
    model: string | undefined;
  }>;
};

export function AgentEditMergedDialog({
  open,
  onOpenChange,
  ctx,
  initialFocus,
  onUpdated,
  onValidate,
  initialValueOverrides,
}: AgentEditMergedDialogProps) {
  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);
  const showDef = hasDefinitionContext(ctx);
  const showInst = hasInstanceContext(ctx);
  const defReadOnly = isDefinitionReadOnly(ctx);

  // ── Queries / mutations ───────────────────────────────────────────────────
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const runtimes = runtimesQuery.data ?? [];
  const runtimeCatalogStatus = runtimesQuery.isLoading
    ? ("loading" as const)
    : runtimesQuery.isError
      ? ("error" as const)
      : ("ready" as const);

  const updatePersonaMutation = useUpdatePersonaMutation();
  const updatePersonaAndPublishMutation =
    useUpdatePersonaAndPublishMutation(null);
  const updateMutation = useUpdateManagedAgentMutation();
  const startMutation = useStartManagedAgentMutation();

  const { data: bakedEnvKeys } = useBakedBuildEnvKeysQuery({ enabled: open });
  const { data: agentAccessOwnerOnly } = useAgentAccessOwnerOnlyQuery({
    enabled: open && showInst,
  });

  const personasQuery = usePersonasQuery();
  const linkedPersona = React.useMemo(
    () =>
      inst?.personaId
        ? (personasQuery.data?.find((p) => p.id === inst.personaId) ?? null)
        : null,
    [inst?.personaId, personasQuery.data],
  );

  // ── Form state — seeded from AgentFormModel ───────────────────────────────
  // D-fields
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [systemPrompt, setSystemPrompt] = React.useState("");
  const [namePoolText, setNamePoolText] = React.useState("");
  // Runtime/model/provider
  const [selectedRuntimeId, setSelectedRuntimeId] = React.useState("custom");
  const [model, setModel] = React.useState("");
  const [provider, setProvider] = React.useState("");
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const [isCustomProviderEditing, setIsCustomProviderEditing] =
    React.useState(false);
  const runtimeTouched = React.useRef(false);

  // I-fields
  const [instanceName, setInstanceName] = React.useState("");
  const [respondTo, setRespondTo] = React.useState<RespondToMode>(
    inst?.respondTo ?? "anyone",
  );
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    inst?.respondToAllowlist ?? [],
  );
  const [parallelism, setParallelism] = React.useState(
    String(inst?.parallelism ?? 1),
  );
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>(
    def?.envVars ?? inst?.envVars ?? {},
  );
  const [instanceEnvVars, setInstanceEnvVars] = React.useState<EnvVarsValue>(
    inst?.envVars ?? {},
  );
  // Harness-pin (I-field)
  const [agentCommand, setAgentCommand] = React.useState(
    inst?.agentCommand ?? "",
  );
  const [inheritHarness, setInheritHarness] = React.useState(
    inst != null && inst.personaId != null && inst.agentCommandOverride == null,
  );
  const [agentArgs, setAgentArgs] = React.useState(
    inst?.agentArgs.join(",") ?? "",
  );
  const [acpCommand, setAcpCommand] = React.useState(inst?.acpCommand ?? "");
  const [originalAgentCommand, setOriginalAgentCommand] = React.useState(
    inst?.agentCommand ?? "",
  );

  // L-fields
  const [autoRestartOnConfigChange, setAutoRestartOnConfigChange] =
    React.useState(inst?.autoRestartOnConfigChange ?? false);

  // UI state
  const [showAdvancedFields, setShowAdvancedFields] = React.useState(false);
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const [aiDefaultsOpen, setAiDefaultsOpen] = React.useState(false);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
  const [isAddHarnessOpen, setIsAddHarnessOpen] = React.useState(false);

  // ── Seed form on open ─────────────────────────────────────────────────────
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — only re-seed on open/entity-switch
  React.useEffect(() => {
    if (!open) return;

    const seed = seedAgentFormModel(ctx);
    setDisplayName(seed.displayName);
    setAvatarUrl(seed.avatarUrl);
    setSystemPrompt(seed.systemPrompt);
    setNamePoolText(formatPersonaNamePoolText(seed.namePool ?? []));
    setModel(seed.model ?? "");
    setProvider(seed.provider ?? "");
    setIsCustomModelEditing(false);
    setIsCustomProviderEditing(false);
    setRespondTo((seed.respondTo ?? "anyone") as RespondToMode);
    setRespondToAllowlist(seed.respondToAllowlist);
    setParallelism(String(seed.parallelism ?? 1));
    setEnvVars(seed.envVars);
    setInstanceEnvVars(seed.instanceEnvVars ?? {});
    setInstanceName(seed.instanceName ?? "");
    setAutoRestartOnConfigChange(seed.autoRestartOnConfigChange ?? false);
    setShowAdvancedFields(false);
    setIsAvatarUploadPending(false);
    runtimeTouched.current = false;

    if (inst) {
      setAgentCommand(inst.agentCommand);
      setOriginalAgentCommand(inst.agentCommand);
      setInheritHarness(
        inst.personaId != null && inst.agentCommandOverride == null,
      );
      setAgentArgs(inst.agentArgs.join(","));
      setAcpCommand(inst.acpCommand);
      const matched =
        runtimes.find((r) => r.command?.trim() === inst.agentCommand.trim()) ??
        runtimes.find((r) => r.id === inst.agentCommand.trim());
      setSelectedRuntimeId(matched ? matched.id : "custom");
    } else if (def) {
      // definition-only: seed runtime from definition
      const defRuntimeId = def.runtime?.trim() ?? "";
      setSelectedRuntimeId(defRuntimeId || "custom");
    }

    // Apply any caller-supplied initial value overrides (R6 review mode).
    if (initialValueOverrides) {
      if (initialValueOverrides.displayName !== undefined)
        setDisplayName(initialValueOverrides.displayName);
      if (initialValueOverrides.systemPrompt !== undefined)
        setSystemPrompt(initialValueOverrides.systemPrompt);
      if (initialValueOverrides.model !== undefined)
        setModel(initialValueOverrides.model ?? "");
      if (initialValueOverrides.provider !== undefined)
        setProvider(initialValueOverrides.provider ?? "");
      if (initialValueOverrides.runtime !== undefined)
        setSelectedRuntimeId(initialValueOverrides.runtime ?? "custom");
    }
  }, [open, inst?.pubkey, def?.id]);

  // Re-derive runtime id when catalog loads
  React.useEffect(() => {
    if (!open || runtimeTouched.current || runtimes.length === 0 || !inst)
      return;
    const matched =
      runtimes.find((r) => r.command?.trim() === inst.agentCommand.trim()) ??
      runtimes.find((r) => r.id === inst.agentCommand.trim());
    if (matched) setSelectedRuntimeId(matched.id);
  }, [open, runtimes, inst?.agentCommand, inst]);

  // ── Runtime / model / provider machinery ──────────────────────────────────
  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(runtimes),
    [runtimes],
  );
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const runtimeDropdownValue = selectedRuntimeId || NO_RUNTIME_DROPDOWN_VALUE;

  // Instance-path runtime dropdown options
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

  // Definition path: uses buildPersonaRuntimeDropdownOptions (supports loading/blank)
  const {
    blankRuntimeOptionLabel: defBlankLabel,
    runtimeDropdownOptions: defRuntimeDropdownOptions,
  } = React.useMemo(() => {
    const result = buildPersonaRuntimeDropdownOptions({
      defaultRuntimeId: getDefaultPersonaRuntime(runtimes)?.id,
      isCreateMode: false,
      runtime: selectedRuntimeId === "custom" ? "" : selectedRuntimeId,
      runtimes,
      runtimesLoading: runtimeCatalogStatus === "loading",
    });
    result.runtimeDropdownOptions.push(ADD_CUSTOM_HARNESS_OPTION);
    return result;
  }, [runtimes, runtimeCatalogStatus, selectedRuntimeId]);

  const originalRuntimeSupportsProvider = React.useMemo(() => {
    if (!inst) return false;
    const originalCommand = originalAgentCommand.trim();
    const matched =
      runtimes.find((r) => r.command?.trim() === originalCommand) ??
      runtimes.find((r) => r.id === originalCommand);
    return runtimeSupportsLlmProviderSelection(matched?.id ?? "");
  }, [runtimes, originalAgentCommand, inst]);

  // Prospective runtime after submit (for gate + discovery)
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

  const llmProviderFieldVisible = runtimeSupportsLlmProviderSelection(
    showInst ? prospectiveRuntimeId : selectedRuntimeId,
  );

  const prospectiveRuntime = runtimes.find(
    (r) => r.id === prospectiveRuntimeId,
  );

  // Resolve inherited env / provider for credential gate
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

  // runtimeFileConfig is used by localModeGate (definition-only). For the
  // merged surface, the credential gate drives Save readiness on instance paths.
  useRuntimeFileConfigQuery(
    showInst ? prospectiveRuntimeId : selectedRuntimeId,
    { enabled: open },
  );

  // Model discovery
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
    selectedRuntime: showInst ? prospectiveRuntime : selectedRuntime,
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
    showInst ? (selectedRuntime?.id ?? "") : selectedRuntimeId,
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
      { id: "", label: getDefaultLlmModelLabel(inheritedModelDefault.value) },
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
      : getDefaultLlmModelLabel(inheritedModelDefault.value),
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
        runtime: showInst ? prospectiveRuntimeId : selectedRuntimeId,
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
    selectedRuntimeId,
    prospectiveRuntimeId,
    showInst,
  ]);

  // ── Submit validity ────────────────────────────────────────────────────────
  const modelRequired = React.useMemo(
    () => isMissingRequiredDropdownField(undefined, model),
    [model],
  );
  const providerRequired = React.useMemo(
    () => isMissingRequiredDropdownField(undefined, provider),
    [provider],
  );

  const providerValid = showInst
    ? isEditAgentProviderSaveValid({
        llmProviderFieldVisible,
        currentProvider: provider,
        originalProvider: inst?.provider ?? null,
        globalProvider: inheritedProviderDefault.value,
        originalRuntimeSupportsProvider,
      })
    : true;

  const parsedParallelism = parseInt(parallelism, 10);

  // ── Submit hook (must come before canSubmit so isSaving is available) ─────
  const {
    isSaving,
    saveError,
    handleSubmit: _handleSubmit,
  } = useAgentEditMergedSubmit({
    ctx,
    displayName,
    avatarUrl,
    systemPrompt,
    namePoolText,
    model,
    provider,
    respondTo,
    respondToAllowlist,
    parallelism,
    parsedParallelism,
    envVars,
    instanceEnvVars,
    instanceName,
    autoRestartOnConfigChange,
    selectedRuntimeId,
    inheritHarness,
    agentCommand,
    agentArgs,
    acpCommand,
    showInst,
    defReadOnly,
    inheritedSubmissionProvider: inheritedSubmission.provider ?? null,
    inheritedSubmissionEnvVars: inheritedSubmission.envVars as Record<
      string,
      string
    >,
    linkedPersonaEnvVars: linkedPersona?.envVars,
    runtimes,
    updatePersona: (p) => updatePersonaMutation.mutateAsync(p),
    updatePersonaAndPublish: (p) =>
      updatePersonaAndPublishMutation.mutateAsync(p),
    updateManagedAgent: (input) => updateMutation.mutateAsync(input),
    startMutate: (pubkey, cbs) => startMutation.mutate(pubkey, cbs),
    onValidate,
    onOpenChange,
    onUpdated,
  });

  const canSubmit =
    !isSaving &&
    !isAvatarUploadPending &&
    displayName.trim().length > 0 &&
    (showInst
      ? providerValid &&
        !requiredEnvKeyMissing &&
        (parsedParallelism > 0 || parallelism === "") &&
        (selectedRuntimeId !== "custom" ||
          inheritHarness ||
          agentCommand.trim().length > 0)
      : true);

  // ── selection helpers ──────────────────────────────────────────────────────
  const selection: RuntimeModelProviderSelection = {
    provider,
    model,
    isCustomProviderEditing,
    isCustomModelEditing,
    envVars: showInst ? instanceEnvVars : envVars,
  };

  function applySelection(next: RuntimeModelProviderSelection) {
    setProvider(next.provider);
    setModel(next.model);
    setIsCustomProviderEditing(next.isCustomProviderEditing);
    setIsCustomModelEditing(next.isCustomModelEditing);
    if (showInst) {
      setInstanceEnvVars(next.envVars);
    } else {
      setEnvVars(next.envVars);
    }
  }

  function handleRuntimeDropdownChange(nextValue: string) {
    const action = runtimeDropdownAction(nextValue);
    if (action.kind === "add-custom-harness") {
      setIsAddHarnessOpen(true);
      return;
    }
    const nextRuntimeId = action.runtimeId;
    const previousRuntimeId = selectedRuntimeId;

    if (showInst) {
      runtimeTouched.current = true;
      const nextRuntime = runtimes.find((r) => r.id === nextRuntimeId);
      const resolvedRuntimeId = nextRuntimeId || "custom";
      setSelectedRuntimeId(resolvedRuntimeId);
      if (resolvedRuntimeId === "custom" || nextRuntime?.command) {
        setInheritHarness(false);
      }
      if (nextRuntime?.command) {
        setAgentCommand(nextRuntime.command);
        setAgentArgs(nextRuntime.defaultArgs.join(","));
      }
    } else {
      setSelectedRuntimeId(nextRuntimeId || "custom");
    }

    applySelection(
      selectionOnRuntimeChange(selection, {
        previousRuntime: previousRuntimeId,
        nextRuntime: nextRuntimeId,
        nextRuntimeCanChooseProvider:
          runtimeSupportsLlmProviderSelection(nextRuntimeId),
        lockedRuntimeReset: "full",
      }),
    );
  }

  const selectSavedHarness = usePendingHarnessSelection(
    runtimes,
    handleRuntimeDropdownChange,
    open,
  );

  function handleProviderDropdownChange(nextValue: string) {
    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
    if (nextProvider === "relay-mesh" && selectedRuntimeId !== "buzz-agent") {
      handleRuntimeDropdownChange("buzz-agent");
    }
    const nextSelection = selectionOnProviderDropdownChange(selection, {
      runtime:
        nextProvider === "relay-mesh"
          ? "buzz-agent"
          : (selectedRuntime?.id ?? selectedRuntimeId),
      nextValue,
      clearModelWhenApiKeyMissing: !showInst,
    });
    applySelection({
      ...nextSelection,
      model: nextProvider === "relay-mesh" ? "auto" : nextSelection.model,
    });
  }

  function handleModelDropdownChange(nextValue: string) {
    applySelection(
      selectionOnModelDropdownChange(selection, {
        nextValue,
        clearKnownModelOnCustomEntry: !showInst,
        isModelCustom: false,
      }),
    );
  }

  // ── Focus target (R2) ──────────────────────────────────────────────────────
  const normalizedFieldFocusFiredRef = React.useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset guard on open/context-switch; pure ref mutation is correct
  React.useEffect(() => {
    normalizedFieldFocusFiredRef.current = false;
  }, [open, inst?.pubkey, def?.id]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — llmProviderFieldVisible is the availability signal; inst?.pubkey handles agent-switch
  React.useEffect(() => {
    if (!open || !initialFocus) return;
    if (initialFocus.type !== "normalized_field") return;
    if (normalizedFieldFocusFiredRef.current) return;
    const targetId =
      initialFocus.field === "provider"
        ? "edit-agent-llm-provider"
        : "edit-agent-model";
    const el = document.getElementById(targetId);
    if (!(el instanceof HTMLElement)) return;
    normalizedFieldFocusFiredRef.current = true;
    const id = requestAnimationFrame(() => {
      el.scrollIntoView({ block: "nearest" });
      el.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [open, initialFocus, inst?.pubkey, llmProviderFieldVisible]);

  // ── Render ─────────────────────────────────────────────────────────────────
  function handleSubmit() {
    void _handleSubmit(canSubmit);
  }

  const previewLabel = displayName.trim() || "Agent name";
  const previewAvatarUrl = avatarUrl.trim() || null;

  const dialogTitle = inst
    ? `Edit ${inst.name}`
    : def
      ? `Edit ${def.displayName}`
      : "Edit agent";

  return (
    <Dialog
      onOpenChange={(next) => {
        if (!isSaving) onOpenChange(next);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-3xl border-0"
        contentClassName="pt-3"
        data-testid="edit-agent-dialog"
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title={dialogTitle}
        footer={
          <div className="flex w-full items-center justify-end gap-2">
            <Button
              disabled={isSaving || isAvatarUploadPending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              data-testid="edit-agent-dialog-submit"
              disabled={!canSubmit}
              onClick={() => void handleSubmit()}
              type="button"
            >
              {isSaving ? "Saving..." : "Save changes"}
            </Button>
          </div>
        }
      >
        <div className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]">
          {/* ── Identity column: avatar + preview ────────────────────── */}
          <div className="flex flex-col items-center gap-2">
            <AgentCreationPreview
              avatarUrl={previewAvatarUrl}
              disabled={isSaving || isAvatarUploadPending || defReadOnly}
              label={previewLabel}
              onClearAvatar={() => setAvatarUrl("")}
              onUploadPendingChange={setIsAvatarUploadPending}
              onSelectAvatar={setAvatarUrl}
            />
          </div>

          {/* ── Main form column ──────────────────────────────────────── */}
          <div className="space-y-5">
            {/* Team-managed notice */}
            {defReadOnly && showDef ? (
              <p
                className="text-sm text-muted-foreground"
                data-testid="team-managed-notice"
              >
                This agent is managed by a team. Its configuration cannot be
                edited here.
              </p>
            ) : null}

            {/* Built-in notice */}
            {def?.isBuiltIn && !defReadOnly ? (
              <p
                className="text-sm text-muted-foreground"
                data-testid="builtin-notice"
              >
                Built-in agent — fully editable.
              </p>
            ) : null}

            {/* ── D-section: Identity + Behavior + Runtime ──────────── */}
            {showDef ? (
              <AgentEditMergedDSection
                defReadOnly={defReadOnly}
                isSaving={isSaving}
                displayName={displayName}
                onDisplayNameChange={setDisplayName}
                systemPrompt={systemPrompt}
                onSystemPromptChange={setSystemPrompt}
                runtimeCatalogStatus={runtimeCatalogStatus}
                runtimeDropdownValue={runtimeDropdownValue}
                defRuntimeDropdownOptions={defRuntimeDropdownOptions}
                defBlankLabel={defBlankLabel}
                onRuntimeChange={handleRuntimeDropdownChange}
                llmProviderFieldVisible={llmProviderFieldVisible}
                providerSelectValue={providerSelectValue}
                providerDropdownOptions={providerDropdownOptions}
                onProviderChange={handleProviderDropdownChange}
                isCustomProviderEditing={isCustomProviderEditing}
                provider={provider}
                onProviderTextChange={setProvider}
                modelSelectValue={modelSelectValue}
                modelDropdownOptions={modelDropdownOptions}
                onModelChange={handleModelDropdownChange}
                modelDiscoveryLoading={modelDiscoveryLoading}
                showCustomModelInput={showCustomModelInput}
                model={model}
                onModelTextChange={setModel}
                modelStatusMessage={modelStatusMessage}
              />
            ) : null}

            {/* ── I-section: Instance + Runtime (when instance present) ── */}
            {showInst && inst ? (
              <AgentEditMergedInstanceSection
                inst={inst}
                showDef={showDef}
                defReadOnly={defReadOnly}
                isSaving={isSaving}
                runtimeCatalogStatus={runtimeCatalogStatus}
                instanceName={instanceName}
                onInstanceNameChange={setInstanceName}
                respondTo={respondTo}
                respondToAllowlist={respondToAllowlist}
                onRespondToChange={setRespondTo}
                onAllowlistChange={setRespondToAllowlist}
                agentAccessOwnerOnly={agentAccessOwnerOnly}
                selectedRuntimeId={selectedRuntimeId}
                runtimeDropdownValue={runtimeDropdownValue}
                instanceRuntimeDropdownOptions={instanceRuntimeDropdownOptions}
                onRuntimeChange={handleRuntimeDropdownChange}
                selectedRuntime={selectedRuntime}
                inheritHarness={inheritHarness}
                agentCommand={agentCommand}
                onAgentCommandChange={setAgentCommand}
                isAddHarnessOpen={isAddHarnessOpen}
                onAddHarnessOpenChange={setIsAddHarnessOpen}
                onSelectSavedHarness={selectSavedHarness}
                llmProviderFieldVisible={llmProviderFieldVisible}
                providerSelectValue={providerSelectValue}
                providerDropdownOptions={providerDropdownOptions}
                onProviderChange={handleProviderDropdownChange}
                isCustomProviderEditing={isCustomProviderEditing}
                provider={provider}
                onProviderTextChange={setProvider}
                providerRequired={providerRequired}
                topLevelSecretEnvVar={topLevelSecretEnvVar}
                apiKeyValue={apiKeyValue}
                apiKeyIsInherited={apiKeyIsInherited}
                apiKeyInheritedLabel={apiKeyInheritedLabel}
                apiKeyIsRequired={apiKeyIsRequired}
                effectiveProvider={effectiveProvider}
                onApiKeyChange={(key, value) => {
                  setInstanceEnvVars((prev) => ({ ...prev, [key]: value }));
                }}
                modelSelectValue={modelSelectValue}
                modelDropdownOptions={modelDropdownOptions}
                onModelChange={handleModelDropdownChange}
                modelDiscoveryLoading={modelDiscoveryLoading}
                showCustomModelInput={showCustomModelInput}
                model={model}
                onModelTextChange={setModel}
                modelStatusMessage={modelStatusMessage}
                modelRequired={modelRequired}
                inheritedSubmission={inheritedSubmission}
                inheritedModelDefault={inheritedModelDefault}
                inheritedProviderDefault={inheritedProviderDefault}
                aiDefaultsOpen={aiDefaultsOpen}
                onAiDefaultsOpenChange={setAiDefaultsOpen}
                aiDefaultsTriggerRef={aiDefaultsTriggerRef}
                showAdvancedFields={showAdvancedFields}
                onShowAdvancedFieldsChange={setShowAdvancedFields}
                acpCommand={acpCommand}
                agentArgs={agentArgs}
                autoRestartOnConfigChange={autoRestartOnConfigChange}
                instanceEnvVars={instanceEnvVars}
                fileSatisfiedEnvKeys={fileSatisfiedEnvKeys}
                advancedRequiredEnvKeys={advancedRequiredEnvKeys}
                initialFocus={initialFocus}
                inheritedEnvVarsForAdvanced={inheritedEnvVarsForAdvanced}
                linkedPersona={linkedPersona}
                prospectiveRuntimeId={prospectiveRuntimeId}
                prospectiveRuntime={prospectiveRuntime}
                parallelism={parallelism}
                onAcpCommandChange={setAcpCommand}
                onAgentArgsChange={setAgentArgs}
                onAutoRestartChange={setAutoRestartOnConfigChange}
                onEnvVarsChange={setInstanceEnvVars}
                onInheritHarnessChange={setInheritHarness}
                onParallelismChange={setParallelism}
                systemPrompt={systemPrompt}
                onSystemPromptChange={setSystemPrompt}
              />
            ) : null}

            {/* Save error */}
            {saveError ? (
              <p className="text-sm text-destructive">{saveError.message}</p>
            ) : null}
          </div>
        </div>
      </ChooserDialogContent>
    </Dialog>
  );
}

// ── Local helpers ─────────────────────────────────────────────────────────────

function getDefaultLlmModelLabel(model: string): string {
  return model ? `Default (${model})` : "Default model";
}
