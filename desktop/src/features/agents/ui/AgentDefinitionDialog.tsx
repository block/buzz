import * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
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
import { personaSubmitBlock } from "./personaSubmitBlock";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  computeLocalModeGate,
  formatRuntimeOptionLabel,
  getDefaultPersonaRuntime,
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
import { useAgentDialogDefaults } from "./useAgentDialogDefaults";
import { AgentAiDefaultsNotice } from "./AgentAiDefaults";
import { AgentDefaultsDialog } from "./AgentDefaultsDialog";
import {
  AgentAiConfigurationModeField,
  HarnessModelDefaultNotice,
  type AgentAiConfigurationMode,
} from "./AgentAiConfigurationMode";
import {
  agentAiConfigurationModeSatisfied,
  agentAiConfigurationPairForMode,
  initialAgentAiConfigurationMode,
} from "./agentAiConfigurationPolicy";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import { buildRuntimeModelProviderPayload } from "./agentDefinitionSubmitPayload";

type AgentDefinitionDialogProps = {
  open: boolean;
  title: string;
  description: string;
  submitLabel: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  error: Error | null;
  isPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    input: CreatePersonaInput | UpdatePersonaInput,
  ) => Promise<unknown>;
  /** Rendered below the form fields in create mode only ("Where to run"). */
  createRunSection?: React.ReactNode;
  /** Extra create-mode submit gate (e.g. incomplete provider config). */
  createSubmitBlocked?: boolean;
};

const ADVANCED_FIELDS_MOTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

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
  // The seed the draft is diffed against at submit: an untouched quad
  // submits no behavior group, keeping unrelated edits hash-quiet.
  const behaviorSeedRef = React.useRef(emptyPersonaBehaviorDraft);
  // Tracks when the runtime was auto-seeded by the default-runtime effect in
  // edit mode (i.e. the user never explicitly chose a runtime). Used to omit
  // the seeded runtime from the submit payload for builtin definitions whose
  // canonical runtime is null — the sync would revert it anyway.
  const isRuntimeAutoSeededRef = React.useRef(false);
  // Guards the seeding effect so it fires at most once per dialog-open.
  // Without this, clearing runtime back to "" via "No preference" would re-
  // trigger the effect (the `runtime` dep would pass the length guard) and
  // snap the dropdown back to the default — an edit-mode regression.
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
  } = useAgentDialogDefaults({ open });
  const defaultRuntime = React.useMemo(
    () => getDefaultPersonaRuntime(runtimes, globalConfig.preferred_runtime),
    [globalConfig.preferred_runtime, runtimes],
  );
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
    // Item 5: collapsed by default in edit mode — only expand if a non-default
    // behavior value demands attention. Having env vars or a name pool is not
    // sufficient reason to auto-open.
    setShowAdvancedFields(false);
    setIsAvatarUploadPending(false);
    isRuntimeAutoSeededRef.current = false;
    hasSeededForOpenRef.current = false;
  }, [initialValues, open]);

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
      // Edit mode: record that this runtime was auto-seeded so the submit path
      // can omit it from the payload for builtin definitions (canonical runtime
      // null; sync would revert the value anyway). Explicit user changes via
      // the dropdown clear this flag.
      isRuntimeAutoSeededRef.current = true;
    }
  }, [defaultRuntime, initialValues, open, runtime, runtimesLoading]);

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
      // isRuntimeAutoSeededRef and hasSeededForOpenRef are NOT reset here — the
      // [initialValues, open] effect resets both when the dialog re-opens.
    }

    onOpenChange(next);
  }

  async function handleSubmit() {
    // D1: the same localModeSatisfied gate as canSubmit prevents form-submit
    // (Enter) from bypassing a missing credential.
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
  // Required credential env keys for this runtime + provider combination.
  // Used to show required markers on the LLM provider label and amber
  // locked rows in the env vars editor.
  // File-layer config for the selected runtime (e.g. goose config.yaml).
  // Used to silence requirements already satisfied there.
  const { data: runtimeFileConfig, isLoading: fileConfigLoading } =
    useRuntimeFileConfigQuery(runtime, { enabled: open });
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
  const { data: bakedEnvKeys, isLoading: bakedLoading } =
    useBakedBuildEnvKeysQuery({ enabled: open });
  const credentialSettled = !fileConfigLoading && !bakedLoading;
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
  // D1: single boolean for both canSubmit and handleSubmit — never recompose.
  const localModeSatisfied = localModeGate.satisfied;
  // Effective provider: agent value → global fallback → file fallback.
  // Mirrors the chain inside computeLocalModeGate so model-option scoping and
  // model requiredness are consistent with the readiness gate.
  const fileProvider = runtimeFileConfig?.provider?.trim() ?? "";
  const effectiveProvider =
    trimmedProvider || inheritedProviderDefault.value || fileProvider;
  // D2: the top-level API key owns display while the full gate remains intact.
  const apiKeyFieldState = useProviderApiKeyFieldState({
    bakedEnvKeys,
    effectiveEnvVars: envVars,
    envVars,
    fileSatisfiedEnvKeys: localModeGate.fileSatisfiedEnvKeys,
    globalEnvVars: globalConfig.env_vars,
    open,
    provider: effectiveProvider,
    requiredEnvKeys,
    satisfactionSettled: credentialSettled,
    setShowAdvancedFields,
  });
  const {
    advancedRequiredEnvKeys,
    inheritedLabel: apiKeyInheritedLabel,
    isInherited: apiKeyIsInherited,
    isRequired: apiKeyIsRequired,
    secretEnvVar: topLevelSecretEnvVar,
    value: apiKeyValue,
  } = apiKeyFieldState;
  // Provider required-ness is a static property of the field's visibility — it
  // does not change based on whether the field is currently filled. Using the
  // dynamic missingNormalizedFields check would flip the asterisk off once a
  // value is selected, which is incoherent (required means required, not
  // "required until satisfied"). runtimeCanChooseLlmProvider is the authoritative
  // gate: it tracks exactly when the provider picker is shown (Buzz Agent/Goose,
  // plus runtime-less legacy/builtin definitions), so the required marker never
  // drifts from whether Save actually needs a provider.
  const providerIsRequired =
    aiConfigurationMode === "custom" && runtimeCanChooseLlmProvider;
  const modelFieldVisible =
    runtime.trim().length > 0 || blankRuntimeModelProviderEditable;
  const isExplicitModelRequired = aiConfigurationMode === "custom";
  // Gate the provider requirement on the field's actual visibility, not the raw
  // runtime capability. Codex/Claude hide the provider picker (they drive their
  // own provider), so Customize must not require a provider there. But a
  // runtime-less legacy/builtin definition still exposes the picker via
  // blankRuntimeModelProviderEditable, so it must keep requiring a provider —
  // otherwise Save could persist `provider: undefined` despite the visible field.
  const customAiPairSatisfied = agentAiConfigurationModeSatisfied(
    aiConfigurationMode,
    { provider, model },
    runtimeCanChooseLlmProvider,
  );
  const isCreateMode = Boolean(initialValues && !("id" in initialValues));
  const selectedRuntimeIsAvailable =
    runtime.trim().length === 0 ||
    selectedRuntime?.availability === "available";
  // Gate model/provider validity through missingNormalizedFields — single
  // source of truth with the readiness gate so display and Save can't drift.
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

  // Derive the single, deterministic reason the action is disabled from the
  // same gate outputs that feed canSubmit — no policy is recomputed here.
  // Precedence mirrors canSubmit's term order, so the reason is `null` exactly
  // when the form can be submitted (transient Saving/Uploading states aside).
  const submitBlockReason = personaSubmitBlock({
    isPending,
    isAvatarUploadPending,
    displayNameEmpty: displayName.trim().length === 0,
    isCreateMode,
    runtimeChosen: runtime.trim().length > 0,
    runtimeAvailable: selectedRuntimeIsAvailable,
    createBackendBlocked: createSubmitBlocked,
    allowlistEmpty: !personaBehaviorDraftValid(behaviorDraft),
    aiConfigurationMode,
    localModeSatisfied,
    localModeMissingFields: localModeGate.missingNormalizedFields,
    localModeMissingEnvKeys: localModeGate.missingEnvKeys,
    customAiPairSatisfied,
    runtimeNeedsProviderSelection: runtimeCanChooseLlmProvider,
    customProviderEmpty: provider.trim().length === 0,
    customModelEmpty: model.trim().length === 0,
  });

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
  // On internal Block builds, BUZZ_AGENT_PROVIDER is baked in and a boot
  // migration rewrites any persisted Databricks v1 values → v2. Hide the v1
  // option there so it is not offered for new selections. OSS builds have no
  // baked provider, so v1 remains visible.
  const hideProviderIds = React.useMemo(
    () =>
      (bakedEnvKeys ?? []).includes("BUZZ_AGENT_PROVIDER")
        ? BLOCK_BUILD_HIDDEN_PROVIDER_IDS
        : new Set<string>(),
    [bakedEnvKeys],
  );
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
    () => sortPersonaRuntimes(runtimes),
    [runtimes],
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
      disabled: isCreateMode && candidate.availability !== "available",
      label: `${formatRuntimeOptionLabel(candidate)}${
        isCreateMode && candidate.id === defaultRuntime?.id ? " (default)" : ""
      }`,
      value: candidate.id,
    })),
  ];
  if (
    runtime.trim().length > 0 &&
    !runtimeDropdownOptions.some((option) => option.value === runtime)
  ) {
    runtimeDropdownOptions.push({
      label: `${runtime.trim()} (current)`,
      value: runtime.trim(),
    });
  }
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
          <div className="flex w-full items-center justify-between gap-3">
            <div className="flex min-h-9 items-center">
              {submitBlockReason ? (
                <p
                  className="text-2xs text-muted-foreground"
                  data-testid="persona-dialog-submit-reason"
                >
                  {submitBlockReason}
                </p>
              ) : null}
            </div>

            <div className="flex items-center gap-2">
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

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="persona-runtime"
              >
                Agent harness
              </label>
              <PersonaDropdownField
                disabled={isPending || runtimesLoading}
                id="persona-runtime"
                onValueChange={handleRuntimeDropdownChange}
                options={runtimeDropdownOptions}
                placeholder={blankRuntimeOptionLabel}
                value={runtimeDropdownValue}
              />
              {runtimeWarning}
            </div>

            {modelFieldVisible ? (
              <AgentAiConfigurationModeField
                mode={aiConfigurationMode}
                needsProviderSelection={runtimeCanChooseLlmProvider}
                onModeChange={handleAiConfigurationModeChange}
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
                label="API Key"
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
              runtimeCanChooseLlmProvider ? (
                <AgentAiDefaultsNotice
                  onEditDefaults={() => setAiDefaultsOpen(true)}
                  triggerRef={aiDefaultsTriggerRef}
                  explicitModel=""
                  explicitProvider=""
                  inheritedModel={inheritedModelDefault}
                  inheritedProvider={inheritedProviderDefault}
                />
              ) : (
                <HarnessModelDefaultNotice model={runtimeFileConfig?.model} />
              )
            ) : null}

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
