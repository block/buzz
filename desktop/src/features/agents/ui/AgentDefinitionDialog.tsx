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
import { AgentCreationPreview } from "./AgentCreationPreview";
import {
  createGateHarnessId,
  createRuntimeSelectionSatisfied,
  runtimeDropdownOptions as buildRuntimeDropdownOptions,
  runtimeDropdownPlaceholder,
} from "./createRuntimeGate";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { PersonaAdvancedFields } from "./PersonaAdvancedFields";
import { PersonaModelField } from "./PersonaModelField";
import { runtimeAvailabilityWarning } from "./runtimeAvailabilityWarning";
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
  hiddenProviderIdsForBuild,
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
  shouldClearKnownModelForSelectionScope,
} from "./agentConfigOptions";
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
import { MODEL_DISCOVERY_LOADING_VALUE } from "./usePersonaModelDiscovery";
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
  modelFieldStatus,
} from "./agentAiConfigurationPolicy";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import { buildRuntimeModelProviderPayload } from "./agentDefinitionSubmitPayload";
import { AgentDefinitionDialogFooter } from "./AgentDefinitionDialogFooter";
import { AgentDefinitionIdentityFields } from "./AgentDefinitionIdentityFields";
import { AgentLlmProviderField } from "./AgentLlmProviderField";
import { AddCustomHarnessDialog } from "./AddCustomHarnessDialog";
import {
  ADD_CUSTOM_HARNESS_OPTION,
  runtimeDropdownAction,
  usePendingHarnessSelection,
} from "./addCustomHarness";
import { useCreateRuntimeSeed } from "./useCreateRuntimeSeed";
import { useRemoteAwareModelDiscovery } from "./useRemoteAwareModelDiscovery";
import type { RemoteModelDiscoveryView } from "./whereToRunIntent";

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
  /**
   * Rendered below the form fields in create mode only ("Where to run"). A
   * render prop because the section's host model probe must carry this
   * component's unsaved credential env (it reads the global layer itself).
   */
  createRunSection?: (args: { envVars: EnvVarsValue }) => React.ReactNode;
  /** Extra create-mode submit gate (e.g. incomplete provider config). */
  createSubmitBlocked?: boolean;
  /**
   * True when "Where to run" targets a backend provider. The harness then comes
   * from the REMOTE host's catalog, so the local-runtime requirements below do
   * not apply — demanding a locally-installed runtime would make every
   * remote-only harness unsubmittable.
   */
  createRunsRemotely?: boolean;
  /**
   * The picked remote harness's model catalog, read from the HOST. Non-null
   * only for a provider create with a harness picked; it then REPLACES local
   * model discovery, because the local catalog answers for this computer and
   * the agent is not going to run here.
   */
  createRemoteModelDiscovery?: RemoteModelDiscoveryView | null;
  /**
   * Display label of the harness picked from the HOST's catalog. The summary
   * would otherwise name the local default runtime, which for a remote create
   * is a harness on the wrong computer that the deploy will never run.
   */
  createRemoteHarnessLabel?: string | null;
  /**
   * Id of the harness pinned on the HOST, which owns every credential question
   * for a provider create: the deploy writes this agent's env on the host,
   * keyed off the REMOTE command (`deploy.rs::metadata_env`), so the local
   * runtime id — seeded from whatever this computer happens to have installed
   * — names the wrong env contract. The id spaces are identical by
   * construction: the SSH provider's discovery emits the same `goose` /
   * `buzz-agent` keys the local catalog uses. Null until a harness is pinned.
   */
  createRemoteHarnessId?: string | null;
  /** This EDIT's definition backs a provider record — see `createRuntimeSeedAction`. */
  editsProviderRecord?: boolean;
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
  createRunsRemotely = false,
  createRemoteModelDiscovery = null,
  createRemoteHarnessLabel = null,
  createRemoteHarnessId = null,
  editsProviderRecord = false,
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
  //
  // One deliberate exception: shedding the seed for a remote create re-arms
  // this, so returning "Where to run" to this computer seeds the local default
  // again rather than leaving a create that requires a local harness with none.
  // That path cannot collide with the "No preference" case above — an explicit
  // dropdown choice clears `isRuntimeAutoSeededRef`, which the shed requires.
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

  useCreateRuntimeSeed({
    aiConfigurationMode,
    createRunsRemotely,
    editsProviderRecord,
    defaultRuntime,
    hasSeededForOpenRef,
    initialValues,
    isRuntimeAutoSeededRef,
    open,
    runtime,
    runtimesLoading,
    setRuntime,
  });

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
      runsRemotely: createRunsRemotely,
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
      await onSubmit(
        {
          id: initialValues.id,
          ...baseInput,
        },
        {
          publishCatalogUpdates: publishCatalogUpdatesOnSave && hasUserChanges,
        },
      );
      return;
    }

    await onSubmit(baseInput, { publishCatalogUpdates: false });
  }

  function handleSubmitForm(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void handleSubmit();
  }

  const selectedRuntime = runtimes.find((p) => p.id === runtime);
  // The harness whose credential contract this dialog must satisfy — the
  // host's pin for a remote create, the local runtime otherwise. See
  // createGateHarnessId for why the local id is the wrong question remotely.
  const effectiveHarnessId = createGateHarnessId({
    runsRemotely: createRunsRemotely,
    runtime,
    remoteHarnessId: createRemoteHarnessId,
  });
  const blankRuntimeModelProviderEditable =
    initialModelProviderEditableWithoutRuntime && runtime.trim().length === 0;
  const runtimeCanChooseLlmProvider =
    runtimeSupportsLlmProviderSelection(effectiveHarnessId) ||
    blankRuntimeModelProviderEditable;
  const llmProviderFieldVisible =
    (effectiveHarnessId.trim().length > 0 && runtimeCanChooseLlmProvider) ||
    blankRuntimeModelProviderEditable;
  const trimmedProvider = provider.trim();
  // Required credential env keys for this runtime + provider combination.
  // Used to show required markers on the LLM provider label and amber
  // locked rows in the env vars editor.
  // File-layer config for the selected runtime (e.g. goose config.yaml).
  // Used to silence requirements already satisfied there.
  const { data: localRuntimeFileConfig } = useRuntimeFileConfigQuery(runtime, {
    enabled: open && !createRunsRemotely,
  });
  // The file layer reads THIS machine's ~/.config, so it can only answer for a
  // local create. Letting a local goose config.yaml silence a requirement that
  // belongs to a different host would turn a loud create-time block into a
  // silent deploy-time failure. Disabling the query is not enough on its own:
  // a disabled query still hands back whatever another surface cached.
  const runtimeFileConfig = createRunsRemotely
    ? undefined
    : localRuntimeFileConfig;
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
        // Deliberately false even for a remote create. The credential keys this
        // gate demands are the ones the agent's env carries, and a remote
        // deploy writes that env to the host verbatim — so a missing key is
        // just as fatal there, and silencing the gate would ship an agent that
        // deploys and then cannot authenticate.
        isProviderMode: false,
        model,
        provider: trimmedProvider,
        runtimeId: effectiveHarnessId,
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
      effectiveHarnessId,
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
  // A remote create has no local runtime to key the field off — its harness
  // lives on the host — so the host's own catalog makes the field meaningful.
  const modelFieldVisible =
    runtime.trim().length > 0 ||
    blankRuntimeModelProviderEditable ||
    createRemoteModelDiscovery !== null;
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
  // How far the LOCAL catalog gates this create — see createRuntimeGate.ts.
  const runtimeGate = {
    isCreateMode,
    runsRemotely: createRunsRemotely,
    runtime,
    selectedRuntime,
    hasLocalDefaultRuntime: defaultRuntime !== null,
  };
  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = useRemoteAwareModelDiscovery({
    local: {
      envVars,
      globalEnvVars: globalConfig.env_vars,
      isCustomProviderEditing,
      modelFieldVisible,
      open,
      provider: effectiveProvider,
      runtime,
      selectedRuntime,
    },
    remote: createRemoteModelDiscovery,
    onHarnessChange: () => {
      setModel("");
      setIsCustomModelEditing(false);
    },
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
  const { blocked: modelBlocked, status: modelStatus } = modelFieldStatus({
    catalog: discoveredModelOptions,
    discoveryStatus: modelDiscoveryStatus,
    isTypedEntry: isCustomModelEditing && showCustomModelInput,
    model,
  });
  // Gate model/provider validity through missingNormalizedFields — single
  // source of truth with the readiness gate so display and Save can't drift.
  const canSubmit =
    canSubmitPersonaDialog({ displayName, isPending }) &&
    createRuntimeSelectionSatisfied(runtimeGate) &&
    (!isCreateMode || !createSubmitBlocked) &&
    // Crash-loop guard, create AND edit: an empty allowlist would crash
    // every instance minted from this definition at startup.
    personaBehaviorDraftValid(behaviorDraft) &&
    // D1: localModeSatisfied covers both missingNormalizedFields AND
    // missingEnvKeys — credential env keys now block submit, not just display.
    localModeSatisfied &&
    customAiPairSatisfied &&
    !modelBlocked &&
    !isAvatarUploadPending;
  const hideProviderIds = hiddenProviderIdsForBuild(bakedEnvKeys);
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
  const runtimeDropdownOptions = buildRuntimeDropdownOptions({
    defaultRuntimeId: defaultRuntime?.id ?? null,
    gate: runtimeGate,
    runtimes,
    runtimesLoading,
  });
  runtimeDropdownOptions.push(ADD_CUSTOM_HARNESS_OPTION);
  // The host's pick wins outright for a remote create: `runtime` still holds
  // whatever the local seeding effects resolved, and naming that harness in
  // the summary would describe a machine this agent will never run on.
  const runtimeSummaryLabel =
    createRemoteHarnessLabel ??
    (selectedRuntime
      ? formatRuntimeOptionLabel(selectedRuntime)
      : runtime.trim() || "Not configured");
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
  const runtimeWarningText = selectedRuntime
    ? runtimeAvailabilityWarning(selectedRuntime)
    : null;
  const runtimeWarning = runtimeWarningText ? (
    <p className="text-xs text-warning">
      {runtimeWarningText} Visit Settings &gt; Agents to set it up.
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
            {/* First, not last: every field below is scoped by the answer. The
                harness comes from the chosen machine's catalog and the models
                come from that harness, so asking this at the end would mean
                answering the dependent questions against the wrong computer
                and then silently re-scoping them. */}
            {isCreateMode ? createRunSection?.({ envVars }) : null}

            <AgentDefinitionIdentityFields
              disabled={isPending}
              displayName={displayName}
              onDisplayNameChange={setDisplayName}
              onSystemPromptChange={setSystemPrompt}
              systemPrompt={systemPrompt}
            />

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
              {/* A remote create has exactly one harness question, and the
                  host's catalog owns it. This picker lists what is installed
                  on THIS computer, so offering it too would present two
                  harness controls of which only the other one reaches the
                  deploy — and its "not installed, visit Settings" warning
                  describes the wrong machine. */}
              {aiConfigurationMode === "custom" && !createRunsRemotely ? (
                <AgentHarnessField
                  disabled={isPending || runtimesLoading}
                  onValueChange={handleRuntimeDropdownChange}
                  options={runtimeDropdownOptions}
                  placeholder={runtimeDropdownPlaceholder({
                    isCreateMode,
                    runtimesLoading,
                  })}
                  value={runtimeDropdownValue}
                  warning={runtimeWarning}
                />
              ) : null}

              {llmProviderFieldVisible && aiConfigurationMode === "custom" ? (
                <AgentLlmProviderField
                  disabled={isPending}
                  isRequired={providerIsRequired}
                  onCustomProviderChange={setProvider}
                  onProviderValueChange={handleProviderDropdownChange}
                  options={providerDropdownOptions}
                  provider={provider}
                  selectValue={providerSelectValue}
                  showCustomInput={showCustomProviderInput}
                />
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
                    modelDiscoveryStatus={modelStatus}
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

            <AddCustomHarnessDialog
              onOpenChange={setIsAddHarnessOpen}
              onSaved={selectSavedHarness}
              open={isAddHarnessOpen}
            />

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
