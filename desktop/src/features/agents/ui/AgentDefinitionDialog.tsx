import * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { AgentCreationPreview } from "./AgentCreationPreview";
import { PersonaDropdownField } from "./PersonaDropdownField";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { PersonaAdvancedFields } from "./PersonaAdvancedFields";
import { PersonaModelField } from "./PersonaModelField";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";
import {
  canSubmitPersonaDialog,
  formatPersonaNamePoolText,
  parsePersonaNamePoolText,
} from "./personaDialogState";
import { hasText } from "./personaDialogEnvVars";
import {
  behaviorForSubmit,
  draftFromBehavior,
  emptyPersonaBehaviorDraft,
  personaBehaviorDraftValid,
} from "./personaBehaviorDraft";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  computeLocalModeGate,
  formatRuntimeOptionLabel,
  getDefaultPersonaRuntime,
  getPersonaHiddenProviderIds,
  getPersonaModelOptions,
  getPersonaProviderOptions,
  getRuntimePersonaModelOptions,
  NO_RUNTIME_DROPDOWN_VALUE,
  runtimeSupportsLlmProviderSelection,
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
} from "./agentConfigOptions";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  modelDropdownOptions as buildModelDropdownOptions,
  relayMeshModelPickerState,
} from "./relayMeshModelPicker";
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
import { useBakedBuildEnvKeysQuery, useRuntimeFileConfigQuery } from "../hooks";
import { useDefinitionAgentDialogDefaults } from "./useAgentDialogDefaults";
import { AgentDefaultsDialog } from "./AgentDefaultsDialog";
import { AgentHarnessField } from "./AgentHarnessField";
import {
  AgentAiConfigurationModeField,
  AgentCreateAiDefaultsSummary,
  type AgentAiConfigurationMode,
} from "./AgentAiConfigurationMode";
import {
  agentAiConfigurationModeSatisfied,
  agentAiConfigurationPairForMode,
  initialAgentAiConfigurationMode,
} from "./agentAiConfigurationPolicy";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import { buildRuntimeModelProviderPayload } from "./agentDefinitionSubmitPayload";
import {
  useSelectableAcpRuntimes,
  visibleAcpRuntimeSeedForCreate,
} from "../lib/runtimeVisibilityPreference";
import type { AgentDefinitionDialogProps } from "./AgentDefinitionDialog.types";
import { ADVANCED_FIELDS_MOTION_TRANSITION } from "./agentAdvancedFieldsMotion";

export function AgentDefinitionDialog({
  open,
  title,
  description,
  submitLabel,
  initialValues,
  error,
  isPending,
  runtimes,
  runtimesLoading = false,
  onOpenChange,
  onSubmit,
  createRunSection,
  createSubmitBlocked = false,
}: AgentDefinitionDialogProps) {
  const [displayName, setDisplayName] = React.useState("");
  const [aiDefaultsOpen, setAiDefaultsOpen] = React.useState(false);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [systemPrompt, setSystemPrompt] = React.useState("");
  const [runtime, setRuntime] = React.useState("");
  const [model, setModel] = React.useState("");
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const [provider, setProvider] = React.useState("");
  const [aiConfigurationMode, setAiConfigurationMode] =
    React.useState<AgentAiConfigurationMode>("defaults");
  const [isCustomProviderEditing, setIsCustomProviderEditing] =
    React.useState(false);
  const [namePoolText, setNamePoolText] = React.useState("");
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>({});
  const [behaviorDraft, setBehaviorDraft] = React.useState(
    emptyPersonaBehaviorDraft,
  );
  // Untouched behavior fields submit no group, keeping edits hash-quiet.
  const behaviorSeedRef = React.useRef(emptyPersonaBehaviorDraft);
  // Lets edit-mode builtin definitions omit an untouched auto-seeded runtime.
  const isRuntimeAutoSeededRef = React.useRef(false);
  // Prevent "No preference" from snapping back to the default.
  const hasSeededForOpenRef = React.useRef(false);
  const [showAdvancedFields, setShowAdvancedFields] = React.useState(false);
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const {
    globalConfig,
    inheritedDefaults: {
      provider: inheritedProviderDefault,
      model: inheritedModelDefault,
    },
    inheritedEnvVars: inheritedEnvVarsForAdvanced,
  } = useDefinitionAgentDialogDefaults(initialValues, open);
  const selectableRuntimes = useSelectableAcpRuntimes(runtimes);
  const defaultRuntime = getDefaultPersonaRuntime(
    selectableRuntimes,
    globalConfig.preferred_runtime,
  );
  const isCreateMode = Boolean(initialValues && !("id" in initialValues));
  const shouldReduceMotion = useReducedMotion();
  const initialModelProviderEditableWithoutRuntime = Boolean(
    initialValues &&
      "id" in initialValues &&
      !hasText(initialValues.runtime) &&
      (hasText(initialValues.model) || hasText(initialValues.provider)),
  );

  React.useEffect(() => {
    if (!open || !initialValues) {
      return;
    }

    setDisplayName(initialValues.displayName);
    setAvatarUrl(initialValues.avatarUrl ?? "");
    setSystemPrompt(initialValues.systemPrompt);
    setRuntime(initialValues.runtime ?? "");
    setModel(initialValues.model ?? "");
    setIsCustomModelEditing(false);
    setProvider(initialValues.provider ?? "");
    setAiConfigurationMode(
      initialAgentAiConfigurationMode({
        provider: initialValues.provider ?? "",
        model: initialValues.model ?? "",
      }),
    );
    setIsCustomProviderEditing(false);
    const nextNamePoolText =
      "namePool" in initialValues
        ? formatPersonaNamePoolText(initialValues.namePool)
        : "";
    const nextEnvVars =
      "envVars" in initialValues ? (initialValues.envVars ?? {}) : {};
    const nextBehaviorDraft = draftFromBehavior(initialValues.behavior);
    behaviorSeedRef.current = draftFromBehavior(initialValues.behavior);
    setBehaviorDraft(nextBehaviorDraft);
    setNamePoolText(nextNamePoolText);
    setEnvVars(nextEnvVars);
    // Advanced always starts collapsed and only changes from its toggle.
    setShowAdvancedFields(false);
    setIsAvatarUploadPending(false);
    isRuntimeAutoSeededRef.current = false;
    hasSeededForOpenRef.current = false;
  }, [initialValues, open]);

  React.useEffect(() => {
    if (!open || !initialValues || "id" in initialValues || runtimesLoading) {
      return;
    }
    const seededRuntime = initialValues.runtime?.trim() ?? "";
    const nextRuntime = visibleAcpRuntimeSeedForCreate(
      seededRuntime,
      selectableRuntimes,
      defaultRuntime?.id,
    );
    if (
      !seededRuntime ||
      runtime.trim() !== seededRuntime ||
      nextRuntime === seededRuntime
    ) {
      return;
    }
    setRuntime(nextRuntime);
    setModel("");
    setProvider("");
    setAiConfigurationMode("defaults");
    setIsCustomModelEditing(false);
    setIsCustomProviderEditing(false);
  }, [
    defaultRuntime?.id,
    initialValues,
    open,
    runtime,
    runtimesLoading,
    selectableRuntimes,
  ]);

  React.useEffect(() => {
    if (
      !open ||
      !initialValues ||
      initialValues.runtime?.trim() ||
      runtimesLoading ||
      runtime.trim().length > 0 ||
      defaultRuntime === null ||
      hasSeededForOpenRef.current
    ) {
      return;
    }

    setRuntime(defaultRuntime.id);
    hasSeededForOpenRef.current = true;
    if ("id" in initialValues) {
      // Builtin definitions omit this untouched inferred runtime on submit.
      isRuntimeAutoSeededRef.current = true;
    }
  }, [defaultRuntime, initialValues, open, runtime, runtimesLoading]);

  // Keep an inherited Create runtime synced with defaults saved in-place.
  React.useEffect(() => {
    if (
      !open ||
      !initialValues ||
      "id" in initialValues ||
      initialValues.runtime?.trim() ||
      aiConfigurationMode !== "defaults" ||
      runtimesLoading ||
      defaultRuntime === null ||
      (runtime.trim().length > 0 && !isRuntimeAutoSeededRef.current)
    ) {
      return;
    }

    if (runtime !== defaultRuntime.id) setRuntime(defaultRuntime.id);
    isRuntimeAutoSeededRef.current = true;
    hasSeededForOpenRef.current = true;
  }, [
    aiConfigurationMode,
    defaultRuntime,
    initialValues,
    open,
    runtime,
    runtimesLoading,
  ]);

  // Keep setup guidance reachable when no available runtime can be inherited.
  React.useEffect(() => {
    if (
      open &&
      isCreateMode &&
      !runtimesLoading &&
      defaultRuntime === null &&
      runtime.trim().length === 0
    ) {
      setAiConfigurationMode("custom");
    }
  }, [defaultRuntime, isCreateMode, open, runtime, runtimesLoading]);

  function handleOpenChange(next: boolean) {
    if (!next) {
      setDisplayName("");
      setAvatarUrl("");
      setSystemPrompt("");
      setRuntime("");
      setModel("");
      setIsCustomModelEditing(false);
      setProvider("");
      setAiConfigurationMode("defaults");
      setIsCustomProviderEditing(false);
      setNamePoolText("");
      setEnvVars({});
      setBehaviorDraft(emptyPersonaBehaviorDraft);
      behaviorSeedRef.current = emptyPersonaBehaviorDraft;
      setShowAdvancedFields(false);
      setIsAvatarUploadPending(false);
      // The open-seeding effect resets both refs on the next open.
    }

    onOpenChange(next);
  }

  async function handleSubmit() {
    // Keep Enter submission on the same credential gate as the button.
    if (!initialValues || !localModeSatisfied || !canSubmit) return;

    const {
      runtime: runtimeForSubmit,
      model: modelForSubmit,
      provider: providerForSubmit,
    } = buildRuntimeModelProviderPayload({
      runtime,
      model: aiConfigurationMode === "defaults" ? "" : model,
      provider: aiConfigurationMode === "defaults" ? "" : provider,
      isEditMode: "id" in initialValues,
      isAutoSeeded: isRuntimeAutoSeededRef.current,
      initialPreviousRuntime: initialValues.runtime?.trim() ?? "",
      initialModel: initialValues.model,
      initialProvider: initialValues.provider,
      initialModelProviderEditableWithoutRuntime,
    });
    const namePool = parsePersonaNamePoolText(namePoolText);
    const namePoolInput =
      namePool.length > 0
        ? namePool
        : "namePool" in initialValues
          ? []
          : undefined;
    const baseInput = {
      displayName: displayName.trim(),
      avatarUrl: avatarUrl.trim() || undefined,
      systemPrompt: systemPrompt,
      runtime: runtimeForSubmit,
      model: modelForSubmit,
      provider: providerForSubmit,
      namePool: namePoolInput,
      envVars,
      behavior: behaviorForSubmit(
        behaviorDraft,
        behaviorSeedRef.current,
        "id" in initialValues,
      ),
    };

    if ("id" in initialValues) {
      await onSubmit({
        id: initialValues.id,
        ...baseInput,
      });
      return;
    }

    await onSubmit(baseInput);
  }

  function handleSubmitForm(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void handleSubmit();
  }

  const selectedRuntime = runtimes.find((p) => p.id === runtime);
  const blankRuntimeModelProviderEditable =
    initialModelProviderEditableWithoutRuntime && runtime.trim().length === 0;
  const runtimeCanChooseLlmProvider =
    runtimeSupportsLlmProviderSelection(runtime) ||
    blankRuntimeModelProviderEditable;
  const llmProviderFieldVisible =
    (runtime.trim().length > 0 && runtimeCanChooseLlmProvider) ||
    blankRuntimeModelProviderEditable;
  const trimmedProvider = provider.trim();
  // File config satisfies credentials before the readiness gate renders them.
  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(runtime, {
    enabled: open,
  });
  function handleAiConfigurationModeChange(nextMode: AgentAiConfigurationMode) {
    setAiConfigurationMode(nextMode);
    setIsCustomProviderEditing(false);
    setIsCustomModelEditing(false);
    const nextPair = agentAiConfigurationPairForMode({
      current: { provider, model },
      inherited: runtimeCanChooseLlmProvider
        ? {
            provider: inheritedProviderDefault.value,
            model: inheritedModelDefault.value,
          }
        : { provider: "", model: runtimeFileConfig?.model?.trim() ?? "" },
      mode: nextMode,
      needsProviderSelection: runtimeCanChooseLlmProvider,
    });
    setProvider(nextPair.provider);
    setModel(nextPair.model);
  }
  const { data: bakedEnvKeys } = useBakedBuildEnvKeysQuery({ enabled: open });
  const localModeGate = React.useMemo(
    () =>
      computeLocalModeGate({
        bakedEnvKeys,
        envVars,
        globalEnvVars: globalConfig.env_vars,
        globalProvider: inheritedProviderDefault.value,
        globalModel: inheritedModelDefault.value,
        isProviderMode: false,
        model,
        provider: trimmedProvider,
        runtimeId: runtime,
        runtimeFileConfig,
      }),
    [
      bakedEnvKeys,
      envVars,
      globalConfig.env_vars,
      inheritedModelDefault.value,
      inheritedProviderDefault.value,
      model,
      trimmedProvider,
      runtime,
      runtimeFileConfig,
    ],
  );
  // requiredEnvKeys: the gate already handles baked-, global-, and file-
  // satisfied keys so no further filtering is needed.
  const { requiredEnvKeys } = localModeGate;
  const localModeSatisfied = localModeGate.satisfied;
  // Effective provider: agent value → global fallback → file fallback.
  // Mirrors the chain inside computeLocalModeGate so model-option scoping and
  // model requiredness are consistent with the readiness gate.
  const fileProvider = runtimeFileConfig?.provider?.trim() ?? "";
  const effectiveProvider =
    trimmedProvider || inheritedProviderDefault.value || fileProvider;
  const apiKeyFieldState = useProviderApiKeyFieldState({
    bakedEnvKeys,
    effectiveEnvVars: envVars,
    envVars,
    fileSatisfiedEnvKeys: localModeGate.fileSatisfiedEnvKeys,
    globalEnvVars: globalConfig.env_vars,
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
  const providerIsRequired =
    aiConfigurationMode === "custom" && runtimeCanChooseLlmProvider;
  const modelFieldVisible =
    runtime.trim().length > 0 || blankRuntimeModelProviderEditable;
  const isExplicitModelRequired = aiConfigurationMode === "custom";
  const customAiPairSatisfied = agentAiConfigurationModeSatisfied(
    aiConfigurationMode,
    { provider, model },
    runtimeCanChooseLlmProvider,
  );
  const selectedRuntimeIsAvailable =
    runtime.trim().length === 0 ||
    selectedRuntime?.availability === "available";
  // Keep model/provider validity aligned with the readiness gate.
  const canSubmit =
    canSubmitPersonaDialog({ displayName, isPending }) &&
    (!isCreateMode || runtime.trim().length > 0) &&
    (!isCreateMode || selectedRuntimeIsAvailable) &&
    (!isCreateMode || !createSubmitBlocked) &&
    // Crash-loop guard, create AND edit: an empty allowlist would crash
    // every instance minted from this definition at startup.
    personaBehaviorDraftValid(behaviorDraft) &&
    // D1: localModeSatisfied covers both missingNormalizedFields AND
    // missingEnvKeys — credential env keys now block submit, not just display.
    localModeSatisfied &&
    customAiPairSatisfied &&
    !isAvatarUploadPending;

  // Merge global env as the base layer so credential keys satisfied via global
  // config are available to model discovery — same rationale as in AgentInstanceEditDialog.
  const envVarsForDiscovery = React.useMemo(
    () => ({ ...globalConfig.env_vars, ...envVars }),
    [globalConfig.env_vars, envVars],
  );
  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars: envVarsForDiscovery,
    isCustomProviderEditing,
    modelFieldVisible,
    open,
    // Gate provider by runtime: runtimes that don't support LLM provider
    // selection (codex, claude) must not inherit the global provider — doing
    // so causes them to discover models from the wrong provider.
    provider: runtimeSupportsLlmProviderSelection(runtime)
      ? effectiveProvider
      : "",
    selectedRuntime,
  });
  const staticModelOptions = getPersonaModelOptions(runtime, effectiveProvider);
  const runtimeModelOptions = getRuntimePersonaModelOptions(runtime);
  const {
    isCustom: isModelCustom,
    isRelayMesh,
    options: modelOptions,
    selectValue: modelSelectValue,
    showCustomInput: showCustomModelInput,
  } = relayMeshModelPickerState({
    discoveredOptions: discoveredModelOptions,
    fallbackOptions: staticModelOptions,
    knownOptions: discoveredModelOptions ?? runtimeModelOptions,
    isCustomEditing: isCustomModelEditing,
    model,
    modelFieldVisible,
    provider: effectiveProvider,
  });
  const hideProviderIds = getPersonaHiddenProviderIds({
    bakedEnvKeys: bakedEnvKeys ?? [],
    selectableRuntimes,
    currentRuntimeId: runtime,
    preserveCurrentRuntime: !isCreateMode,
  });
  const providerOptions = getPersonaProviderOptions(
    trimmedProvider,
    runtime,
    inheritedProviderDefault.source === "global"
      ? inheritedProviderDefault.value
      : "",
    hideProviderIds,
  );
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : trimmedProvider || AUTO_PROVIDER_DROPDOWN_VALUE;
  const showCustomProviderInput =
    llmProviderFieldVisible && isCustomProviderEditing;
  const runtimeDropdownValue = runtime.trim() || NO_RUNTIME_DROPDOWN_VALUE;
  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(selectableRuntimes),
    [selectableRuntimes],
  );
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
    ...sortedRuntimes.map((candidate) => ({
      disabled:
        isCreateMode &&
        defaultRuntime !== null &&
        candidate.availability !== "available",
      label: `${formatRuntimeOptionLabel(candidate)}${
        isCreateMode && candidate.id === defaultRuntime?.id ? " (default)" : ""
      }`,
      value: candidate.id,
    })),
  ];
  if (
    !isCreateMode &&
    runtime.trim().length > 0 &&
    !runtimeDropdownOptions.some((option) => option.value === runtime)
  ) {
    runtimeDropdownOptions.push({
      label: `${runtime.trim()} (current)`,
      value: runtime.trim(),
    });
  }
  const runtimeSummaryLabel = selectedRuntime
    ? formatRuntimeOptionLabel(selectedRuntime)
    : runtime.trim() || "Not configured";
  const providerDropdownOptions: PersonaDropdownOption[] = [
    ...providerOptions
      .filter((option) => option.id.trim().length > 0)
      .map((option) => ({
        label: option.label,
        value: option.id,
      })),
    { label: "Custom provider...", value: CUSTOM_PROVIDER_DROPDOWN_VALUE },
  ];
  const modelDropdownOptions: PersonaDropdownOption[] =
    buildModelDropdownOptions({
      allowCustom: !isRelayMesh,
      globalModel: undefined,
      loading: modelDiscoveryLoading && discoveredModelOptions === null,
      loadingValue: MODEL_DISCOVERY_LOADING_VALUE,
      options: modelOptions,
    })
      .filter(
        (option) => isRelayMesh || option.value !== AUTO_MODEL_DROPDOWN_VALUE,
      )
      .map((option) =>
        isRelayMesh && option.value === AUTO_MODEL_DROPDOWN_VALUE
          ? { ...option, label: "Automatic" }
          : option,
      );
  const previewLabel = displayName.trim() || "Agent name";
  const previewAvatarUrl = avatarUrl.trim() || null;
  const runtimeWarning =
    selectedRuntime && selectedRuntime.availability !== "available" ? (
      <p className="text-xs text-warning">
        {selectedRuntime.availability === "adapter_missing"
          ? `${selectedRuntime.label} CLI is installed but the ACP adapter is missing.`
          : selectedRuntime.availability === "adapter_outdated"
            ? `${selectedRuntime.label} ACP adapter is outdated — reinstall to continue.`
            : selectedRuntime.availability === "cli_missing"
              ? `${selectedRuntime.label} ACP adapter is installed but the CLI is missing.`
              : `${selectedRuntime.label} is not installed.`}{" "}
        Visit Settings &gt; Agents to set it up.
      </p>
    ) : null;
  const advancedFieldsTransition = shouldReduceMotion
    ? { duration: 0 }
    : ADVANCED_FIELDS_MOTION_TRANSITION;

  React.useEffect(() => {
    if (
      !open ||
      !modelFieldVisible ||
      isCustomModelEditing ||
      !shouldClearKnownModelForSelectionScope({
        model,
        provider: effectiveProvider,
        runtime,
      })
    ) {
      return;
    }

    setModel("");
    setIsCustomModelEditing(false);
  }, [
    isCustomModelEditing,
    model,
    modelFieldVisible,
    open,
    effectiveProvider,
    runtime,
  ]);

  const selection: RuntimeModelProviderSelection = {
    provider,
    model,
    isCustomProviderEditing,
    isCustomModelEditing,
    envVars,
  };

  function applySelection(next: RuntimeModelProviderSelection) {
    setProvider(next.provider);
    setModel(next.model);
    setIsCustomProviderEditing(next.isCustomProviderEditing);
    setIsCustomModelEditing(next.isCustomModelEditing);
    setEnvVars(next.envVars);
  }

  function handleRuntimeDropdownChange(nextValue: string) {
    const nextRuntime =
      nextValue === NO_RUNTIME_DROPDOWN_VALUE ? "" : nextValue;
    // The user made an explicit choice — no longer auto-seeded.
    isRuntimeAutoSeededRef.current = false;
    setRuntime(nextRuntime);
    applySelection(
      selectionOnRuntimeChange(selection, {
        previousRuntime: runtime,
        nextRuntime,
        nextRuntimeCanChooseProvider:
          nextRuntime.trim().length > 0 &&
          runtimeSupportsLlmProviderSelection(nextRuntime),
        lockedRuntimeReset: "full",
      }),
    );
  }

  function handleProviderDropdownChange(nextValue: string) {
    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
    if (nextProvider === "relay-mesh" && runtime !== "buzz-agent") {
      handleRuntimeDropdownChange("buzz-agent");
    }
    const nextSelection = selectionOnProviderDropdownChange(selection, {
      runtime: nextProvider === "relay-mesh" ? "buzz-agent" : runtime,
      nextValue,
      clearModelWhenApiKeyMissing: true,
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
        clearKnownModelOnCustomEntry: true,
        isModelCustom,
      }),
    );
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && (isPending || isAvatarUploadPending)) return;
        handleOpenChange(nextOpen);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-3xl border-0"
        contentClassName="pt-3"
        data-testid="persona-dialog"
        description={description}
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title={title}
        footer={
          <div className="flex w-full items-center justify-end gap-2">
            <Button
              disabled={isPending || isAvatarUploadPending}
              onClick={() => handleOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              data-testid="persona-dialog-submit"
              disabled={!canSubmit}
              form="persona-dialog-form"
              type="submit"
            >
              {isPending
                ? "Saving..."
                : isAvatarUploadPending
                  ? "Uploading..."
                  : submitLabel}
            </Button>
          </div>
        }
      >
        <form
          className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]"
          id="persona-dialog-form"
          onSubmit={handleSubmitForm}
        >
          <AgentCreationPreview
            avatarUrl={previewAvatarUrl}
            disabled={isPending || isAvatarUploadPending}
            label={previewLabel}
            onClearAvatar={() => setAvatarUrl("")}
            onUploadPendingChange={setIsAvatarUploadPending}
            onSelectAvatar={setAvatarUrl}
          />

          <div className="space-y-5">
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="persona-display-name"
              >
                Agent name
              </label>
              <div
                className={cn(
                  "flex min-h-11 items-center px-3",
                  PERSONA_FIELD_SHELL_CLASS,
                )}
              >
                <Input
                  autoCorrect="off"
                  className={cn(
                    "h-8 px-0 py-0 leading-6",
                    PERSONA_FIELD_CONTROL_CLASS,
                  )}
                  disabled={isPending}
                  id="persona-display-name"
                  onChange={(event) => setDisplayName(event.target.value)}
                  placeholder="Fizz"
                  value={displayName}
                />
              </div>
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="persona-system-prompt"
              >
                Agent instructions
              </label>
              <div className={PERSONA_FIELD_SHELL_CLASS}>
                <Textarea
                  className={cn(
                    "min-h-40 resize-y px-3 py-3 leading-5",
                    PERSONA_FIELD_CONTROL_CLASS,
                  )}
                  disabled={isPending}
                  id="persona-system-prompt"
                  onChange={(event) => setSystemPrompt(event.target.value)}
                  placeholder="Describe what this agent should do."
                  value={systemPrompt}
                />
              </div>
            </div>

            {modelFieldVisible ? (
              <AgentAiConfigurationModeField
                mode={aiConfigurationMode}
                needsProviderSelection={runtimeCanChooseLlmProvider}
                onModeChange={handleAiConfigurationModeChange}
              />
            ) : null}

            <div
              className="space-y-5"
              data-testid={`agent-${aiConfigurationMode}-configuration-section`}
            >
              {aiConfigurationMode === "custom" ? (
                <AgentHarnessField
                  disabled={isPending || runtimesLoading}
                  onValueChange={handleRuntimeDropdownChange}
                  options={runtimeDropdownOptions}
                  placeholder={blankRuntimeOptionLabel}
                  value={runtimeDropdownValue}
                  warning={runtimeWarning}
                />
              ) : null}

              {llmProviderFieldVisible && aiConfigurationMode === "custom" ? (
                <div className="space-y-1.5">
                  <RequiredFieldLabel
                    htmlFor="persona-llm-provider"
                    isRequired={providerIsRequired}
                  >
                    LLM provider
                    {!providerIsRequired ? (
                      <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                        Optional
                      </span>
                    ) : null}
                  </RequiredFieldLabel>
                  <PersonaDropdownField
                    disabled={isPending}
                    id="persona-llm-provider"
                    onValueChange={handleProviderDropdownChange}
                    options={providerDropdownOptions}
                    placeholder="Choose a provider"
                    value={providerSelectValue}
                  />
                  {showCustomProviderInput ? (
                    <div
                      className={cn(
                        "mt-2 flex min-h-11 items-center px-3",
                        PERSONA_FIELD_SHELL_CLASS,
                      )}
                    >
                      <Input
                        aria-label="Custom provider ID"
                        autoCorrect="off"
                        className={cn(
                          "h-8 px-0 py-0 leading-6",
                          PERSONA_FIELD_CONTROL_CLASS,
                        )}
                        disabled={isPending}
                        id="persona-custom-provider"
                        onChange={(event) => setProvider(event.target.value)}
                        placeholder="Custom provider ID"
                        value={provider}
                      />
                    </div>
                  ) : null}
                </div>
              ) : null}

              {llmProviderFieldVisible &&
              aiConfigurationMode === "custom" &&
              topLevelSecretEnvVar ? (
                <PersonaProviderApiKeyField
                  disabled={isPending}
                  isInherited={apiKeyIsInherited}
                  inheritedLabel={apiKeyInheritedLabel}
                  isRequired={apiKeyIsRequired}
                  label={
                    effectiveProvider === "anthropic"
                      ? "Anthropic API key"
                      : "OpenAI API key"
                  }
                  onValueChange={(next) => {
                    setEnvVars((prev) => ({
                      ...prev,
                      [topLevelSecretEnvVar]: next,
                    }));
                  }}
                  value={apiKeyValue}
                />
              ) : null}

              <AnimatePresence initial={false}>
                {modelFieldVisible && aiConfigurationMode === "custom" ? (
                  <PersonaModelField
                    disabled={isPending}
                    isExplicitModelRequired={isExplicitModelRequired}
                    model={model}
                    modelDiscoveryStatus={modelDiscoveryStatus}
                    modelDropdownOptions={modelDropdownOptions}
                    modelSelectValue={modelSelectValue}
                    onCustomModelChange={setModel}
                    showSharedComputeAutoHint={
                      isRelayMesh &&
                      modelSelectValue === AUTO_MODEL_DROPDOWN_VALUE
                    }
                    onModelValueChange={handleModelDropdownChange}
                    showCustomModelInput={showCustomModelInput}
                    transition={advancedFieldsTransition}
                  />
                ) : null}
              </AnimatePresence>

              {aiConfigurationMode === "defaults" ? (
                <AgentCreateAiDefaultsSummary
                  canChooseProvider={runtimeCanChooseLlmProvider}
                  harness={runtimeSummaryLabel}
                  inheritedModel={inheritedModelDefault}
                  inheritedProvider={inheritedProviderDefault}
                  isConfigured={localModeGate.satisfied}
                  model={runtimeFileConfig?.model}
                  onEditDefaults={() => setAiDefaultsOpen(true)}
                  triggerRef={aiDefaultsTriggerRef}
                />
              ) : null}
            </div>

            <AgentDefaultsDialog
              onOpenChange={setAiDefaultsOpen}
              open={runtimeCanChooseLlmProvider && aiDefaultsOpen}
              returnFocusRef={aiDefaultsTriggerRef}
            />

            {isCreateMode ? createRunSection : null}

            <div className="space-y-3">
              <button
                aria-expanded={showAdvancedFields}
                className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                onClick={() => setShowAdvancedFields((current) => !current)}
                type="button"
              >
                <span>Advanced</span>
                {localModeGate.missingEnvKeys.some((key) =>
                  advancedRequiredEnvKeys.includes(key),
                ) ? (
                  <span
                    aria-hidden="true"
                    className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs text-destructive"
                    data-testid="persona-advanced-required-badge"
                  >
                    Required
                  </span>
                ) : null}
                <ChevronDown
                  className={cn(
                    "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
                    showAdvancedFields && "rotate-180",
                  )}
                />
              </button>
              <AnimatePresence initial={false}>
                {showAdvancedFields ? (
                  <motion.div
                    animate={{ height: "auto", opacity: 1, scale: 1 }}
                    className="origin-top overflow-hidden"
                    exit={{ height: 0, opacity: 0, scale: 0.98 }}
                    initial={{ height: 0, opacity: 0, scale: 0.98 }}
                    key="persona-advanced-fields"
                    transition={advancedFieldsTransition}
                  >
                    <PersonaAdvancedFields
                      behaviorDraft={behaviorDraft}
                      disabled={isPending}
                      envVars={envVars}
                      fileSatisfiedEnvKeys={localModeGate.fileSatisfiedEnvKeys}
                      hiddenEnvKeys={
                        topLevelSecretEnvVar ? [topLevelSecretEnvVar] : []
                      }
                      inheritedEnvVars={inheritedEnvVarsForAdvanced}
                      model={model}
                      modelTuningRuntimeId={runtime}
                      namePoolText={namePoolText}
                      onBehaviorDraftChange={setBehaviorDraft}
                      onEnvVarsChange={setEnvVars}
                      onNamePoolTextChange={setNamePoolText}
                      provider={effectiveProvider}
                      requiredEnvKeys={advancedRequiredEnvKeys}
                    />
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>

            {error ? (
              <p className="text-sm text-destructive">{error.message}</p>
            ) : null}
          </div>
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
