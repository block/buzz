import * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { AgentCreationPreview } from "./AgentCreationPreview";
import { pushServerPromptUpdateAfterSubmit } from "./serverPromptUpdatePush";
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
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  buildPersonaRuntimeDropdownOptions,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  computeLocalModeGate,
  localModeGateSatisfiedForSubmit,
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
  shouldClearKnownModelForSelectionScope,
} from "./agentConfigOptions";
import { relayMeshModelPickerState } from "./relayMeshModelPicker";
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
import { AgentDefinitionDialogFooter } from "./AgentDefinitionDialogFooter";
import { AddCustomHarnessDialog } from "./AddCustomHarnessDialog";
import {
  ADD_CUSTOM_HARNESS_OPTION,
  runtimeDropdownAction,
  usePendingHarnessSelection,
} from "./addCustomHarness";
import { useServerAgents } from "../useServerAgents";
import { slugFromName } from "../spawnerPreference";
import { LlmProviderField } from "./LlmProviderField";
import {
  buildPersonaModelDropdownOptions,
  PersonaRuntimeWarning,
} from "./personaRuntimeDropdown";
import { ServerModelField } from "./ServerModelField";
import { ServerRunsOnBanner } from "./ServerRunsOnBanner";
import {
  useServerAgentEditContext,
  withCurrentValueOption,
} from "./useServerAgentEditContext";

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
    options: AgentDefinitionSubmitOptions,
  ) => Promise<unknown>;
  /** Publishes saved changes when the edited agent is shared in the catalog. */
  publishCatalogUpdatesOnSave?: boolean;
  /** Rendered below the form fields in create mode only ("Where to run"). */
  createRunSection?: React.ReactNode;
  /** Extra create-mode submit gate (e.g. incomplete provider config). */
  createSubmitBlocked?: boolean;
};

export type AgentDefinitionSubmitOptions = {
  publishCatalogUpdates: boolean;
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
  publishCatalogUpdatesOnSave = false,
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
  const [hasUserChanges, setHasUserChanges] = React.useState(false);
  const [isAddHarnessOpen, setIsAddHarnessOpen] = React.useState(false);
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
    setHasUserChanges(false);
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
      setHasUserChanges(false);
      setIsAddHarnessOpen(false);
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
      const result = await onSubmit(
        {
          id: initialValues.id,
          ...baseInput,
        },
        {
          publishCatalogUpdates: publishCatalogUpdatesOnSave && hasUserChanges,
        },
      );
      await pushServerPromptUpdateAfterSubmit(serverContext, result, baseInput);
      return;
    }

    await onSubmit(baseInput, { publishCatalogUpdates: false });
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
  // Server residency: a definition deployed to a spawner is configured there,
  // so the harness belongs to the host and the model catalog is the one it
  // advertises. Matched on the *initial* name — the slug the spec was published
  // under does not change while the user is retyping the display name.
  const { agents: serverAgents } = useServerAgents();
  const deployedServerAgent = React.useMemo(() => {
    if (isCreateMode || !initialValues) return null;
    const slug = slugFromName(initialValues.displayName);
    if (!slug) return null;
    return serverAgents.find((candidate) => candidate.slug === slug) ?? null;
  }, [isCreateMode, initialValues, serverAgents]);
  const server = useServerAgentEditContext({
    relocatedToSpawner: null,
    deployedSpawnerPubkey: deployedServerAgent?.spawnerPubkey ?? null,
    agentPubkey: deployedServerAgent?.status.agentPubkey ?? null,
    slug: deployedServerAgent?.slug ?? null,
    provider: trimmedProvider,
  });
  const serverContext = server.context;
  const serverAi = server.ai;

  // Required credential env keys for this runtime + provider combination.
  // Used to show required markers on the LLM provider label and amber
  // locked rows in the env vars editor.
  // File-layer config for the selected runtime (e.g. goose config.yaml).
  // Used to silence requirements already satisfied there.
  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(runtime, {
    enabled: open,
  });
  function handleAiConfigurationModeChange(nextMode: AgentAiConfigurationMode) {
    setHasUserChanges(true);
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
  // Server-hosted definitions authenticate on the spawner (e.g. OAuth) — a
  // credential key missing on this machine must not block Save there.
  const localModeSatisfied = localModeGateSatisfiedForSubmit(
    localModeGate,
    serverContext !== null,
  );
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
    serverManaged: serverContext !== null,
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
  const { blankRuntimeOptionLabel, runtimeDropdownOptions } =
    buildPersonaRuntimeDropdownOptions({
      defaultRuntimeId: defaultRuntime?.id,
      isCreateMode,
      runtime,
      runtimes,
      runtimesLoading,
    });
  runtimeDropdownOptions.push(ADD_CUSTOM_HARNESS_OPTION);
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
  const modelDropdownOptions = buildPersonaModelDropdownOptions({
    isRelayMesh,
    loading: modelDiscoveryLoading && discoveredModelOptions === null,
    loadingValue: MODEL_DISCOVERY_LOADING_VALUE,
    options: modelOptions,
  });
  const previewLabel = displayName.trim() || "Agent name";
  const previewAvatarUrl = avatarUrl.trim() || null;
  const advancedFieldsTransition = shouldReduceMotion
    ? { duration: 0 }
    : ADVANCED_FIELDS_MOTION_TRANSITION;

  React.useEffect(() => {
    if (
      !open ||
      serverContext ||
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
    serverContext,
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
    const action = runtimeDropdownAction(nextValue);
    if (action.kind === "add-custom-harness") {
      setIsAddHarnessOpen(true);
      return;
    }
    setHasUserChanges(true);
    const nextRuntime = action.runtimeId;
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

  // Routed through the normal change handler so a harness registered inline
  // resets model/provider exactly as a hand-picked one would. Scoped to `open`
  // so a pending id can't outlive the dialog that started the registration.
  const selectSavedHarness = usePendingHarnessSelection(
    runtimes,
    handleRuntimeDropdownChange,
    open,
  );

  function handleProviderDropdownChange(nextValue: string) {
    setHasUserChanges(true);
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
    setHasUserChanges(true);
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
          <AgentDefinitionDialogFooter
            canSubmit={canSubmit}
            isAvatarUploadPending={isAvatarUploadPending}
            isPending={isPending}
            onCancel={() => handleOpenChange(false)}
            publishesCatalogUpdates={
              publishCatalogUpdatesOnSave && hasUserChanges
            }
            submitBlockReason={null}
            submitLabel={submitLabel}
          />
        }
      >
        <form
          className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]"
          id="persona-dialog-form"
          onChangeCapture={() => setHasUserChanges(true)}
          onSubmit={handleSubmitForm}
        >
          <AgentCreationPreview
            avatarUrl={previewAvatarUrl}
            disabled={isPending || isAvatarUploadPending}
            label={previewLabel}
            onClearAvatar={() => {
              setHasUserChanges(true);
              setAvatarUrl("");
            }}
            onUploadPendingChange={setIsAvatarUploadPending}
            onSelectAvatar={(nextAvatarUrl) => {
              setHasUserChanges(true);
              setAvatarUrl(nextAvatarUrl);
            }}
          />

          <div className="space-y-5">
            {serverContext ? (
              <ServerRunsOnBanner
                pendingUpdate={server.pendingUpdate}
                runtime={server.runtime}
                spawnerName={serverContext.spawnerName}
              />
            ) : null}

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
              {aiConfigurationMode === "custom" && !serverContext ? (
                <AgentHarnessField
                  disabled={isPending || runtimesLoading}
                  onValueChange={handleRuntimeDropdownChange}
                  options={runtimeDropdownOptions}
                  placeholder={blankRuntimeOptionLabel}
                  value={runtimeDropdownValue}
                  warning={<PersonaRuntimeWarning runtime={selectedRuntime} />}
                />
              ) : null}

              {aiConfigurationMode === "custom" &&
              (serverContext ? serverAi !== null : llmProviderFieldVisible) ? (
                <LlmProviderField
                  customInputId="persona-custom-provider"
                  disabled={isPending}
                  id="persona-llm-provider"
                  isRequired={providerIsRequired}
                  onProviderTextChange={setProvider}
                  onValueChange={handleProviderDropdownChange}
                  options={
                    serverContext && serverAi
                      ? withCurrentValueOption(server.providerOptions, provider)
                      : providerDropdownOptions
                  }
                  placeholder="Choose a provider"
                  providerValue={provider}
                  selectValue={
                    serverContext ? trimmedProvider : providerSelectValue
                  }
                  showCustomInput={!serverContext && showCustomProviderInput}
                />
              ) : null}

              {llmProviderFieldVisible &&
              aiConfigurationMode === "custom" &&
              topLevelSecretEnvVar &&
              !serverContext ? (
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

              {serverContext &&
              !serverAi &&
              aiConfigurationMode === "custom" ? (
                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="persona-server-model"
                  >
                    Model
                  </label>
                  <ServerModelField
                    disabled={isPending}
                    id="persona-server-model"
                    onChange={setModel}
                    value={model}
                  />
                </div>
              ) : null}

              <AnimatePresence initial={false}>
                {(serverContext ? serverAi !== null : modelFieldVisible) &&
                aiConfigurationMode === "custom" ? (
                  <PersonaModelField
                    disabled={isPending}
                    isExplicitModelRequired={isExplicitModelRequired}
                    model={model}
                    modelDiscoveryStatus={modelDiscoveryStatus}
                    modelDropdownOptions={
                      serverContext && serverAi
                        ? withCurrentValueOption(server.modelOptions, model)
                        : modelDropdownOptions
                    }
                    modelSelectValue={
                      serverContext && serverAi
                        ? model.trim()
                        : modelSelectValue
                    }
                    onCustomModelChange={setModel}
                    showSharedComputeAutoHint={
                      isRelayMesh &&
                      modelSelectValue === AUTO_MODEL_DROPDOWN_VALUE
                    }
                    onModelValueChange={handleModelDropdownChange}
                    showCustomModelInput={
                      serverContext ? false : showCustomModelInput
                    }
                    transition={advancedFieldsTransition}
                  />
                ) : null}
              </AnimatePresence>

              {aiConfigurationMode === "defaults" && !serverContext ? (
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

              {serverContext ? (
                <p className="text-xs text-muted-foreground">
                  Applied on the server. Saving restarts the agent.
                </p>
              ) : null}
            </div>

            <AgentDefaultsDialog
              onOpenChange={setAiDefaultsOpen}
              open={runtimeCanChooseLlmProvider && aiDefaultsOpen}
              returnFocusRef={aiDefaultsTriggerRef}
            />

            <AddCustomHarnessDialog
              onOpenChange={setIsAddHarnessOpen}
              onSaved={selectSavedHarness}
              open={isAddHarnessOpen}
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
                      onBehaviorDraftChange={(nextBehaviorDraft) => {
                        setHasUserChanges(true);
                        setBehaviorDraft(nextBehaviorDraft);
                      }}
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
