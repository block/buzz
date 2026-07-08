import * as React from "react";
import { ChevronDown, RefreshCw, Upload } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import { useWindowFileDragOver } from "./useWindowFileDragOver";
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
  getImportButtonLabel,
  getImportButtonTone,
  getImportErrorLabel,
  IMPORT_ERROR_VISIBILITY_MS,
} from "./personaDialogImportState";
import {
  canSubmitPersonaDialog,
  formatPersonaNamePoolText,
  parsePersonaNamePoolText,
} from "./personaDialogState";
import {
  getAdvancedEnvVars,
  hasAdvancedEnvVars,
  hasText,
} from "./personaDialogEnvVars";
import {
  behaviorForSubmit,
  draftFromBehavior,
  emptyPersonaBehaviorDraft,
  personaBehaviorDraftValid,
} from "./personaBehaviorDraft";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  computeLocalModeGate,
  formatRuntimeOptionLabel,
  getDefaultLlmProviderLabel,
  getDefaultPersonaRuntime,
  getModelSelectValue,
  getPersonaModelOptions,
  getPersonaProviderOptions,
  getProviderApiKeyConfig,
  getProviderApiKeyEnvVar,
  getRuntimePersonaModelOptions,
  hasPersonaModelOption,
  NO_RUNTIME_DROPDOWN_VALUE,
  providerRequiresExplicitModel,
  runtimeSupportsLlmProviderSelection,
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
} from "./personaDialogPickers";
import { RequiredFieldLabel } from "./personaProviderModelFields";
import {
  envVarsMergingAdvancedEdit,
  envVarsWithProviderApiKey,
} from "./providerEnvVarUpdates";
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
import { useGlobalAgentConfig } from "../useGlobalAgentConfig";
import { isBuzzAgentRuntime } from "./buzzAgentConfig";

type AgentDefinitionDialogProps = {
  open: boolean;
  title: string;
  description: string;
  submitLabel: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  error: Error | null;
  isPending: boolean;
  isImportPending?: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    input: CreatePersonaInput | UpdatePersonaInput,
  ) => Promise<unknown>;
  onImportUpdateFile?: (
    personaId: string,
    fileBytes: number[],
    fileName: string,
  ) => Promise<void>;
  /**
   * Rendered in the footer's left slot in create mode only — edit mode's
   * import button owns that slot (`canImportPersonaUpdate`).
   */
  createFooterSlot?: React.ReactNode;
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
  isImportPending = false,
  runtimes,
  runtimesLoading = false,
  onOpenChange,
  onSubmit,
  onImportUpdateFile,
  createFooterSlot,
  createRunSection,
  createSubmitBlocked = false,
}: AgentDefinitionDialogProps) {
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [systemPrompt, setSystemPrompt] = React.useState("");
  const [runtime, setRuntime] = React.useState("");
  const [model, setModel] = React.useState("");
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const [provider, setProvider] = React.useState("");
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
  const [showAdvancedFields, setShowAdvancedFields] = React.useState(false);
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const [isImportingUpdate, setIsImportingUpdate] = React.useState(false);
  const [importErrorMessage, setImportErrorMessage] = React.useState<
    string | null
  >(null);
  const { globalConfig } = useGlobalAgentConfig();
  const isEditMode = Boolean(initialValues && "id" in initialValues);
  const editPersonaId =
    isEditMode && initialValues && "id" in initialValues
      ? initialValues.id
      : null;
  const canImportPersonaUpdate = isEditMode && Boolean(onImportUpdateFile);
  const defaultRuntime = React.useMemo(
    () => getDefaultPersonaRuntime(runtimes),
    [runtimes],
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
    setIsCustomProviderEditing(false);
    const nextNamePoolText =
      "namePool" in initialValues
        ? formatPersonaNamePoolText(initialValues.namePool)
        : "";
    const nextEnvVars =
      "envVars" in initialValues ? (initialValues.envVars ?? {}) : {};
    const managedApiKeyEnvVar = getProviderApiKeyEnvVar(
      initialValues.provider ?? "",
    );
    const nextBehaviorDraft = draftFromBehavior(initialValues.behavior);
    behaviorSeedRef.current = draftFromBehavior(initialValues.behavior);
    setBehaviorDraft(nextBehaviorDraft);
    setNamePoolText(nextNamePoolText);
    setEnvVars(nextEnvVars);
    setShowAdvancedFields(
      nextNamePoolText.trim().length > 0 ||
        hasAdvancedEnvVars(nextEnvVars, managedApiKeyEnvVar) ||
        nextBehaviorDraft.respondTo !== null ||
        nextBehaviorDraft.mcpToolsets.trim().length > 0 ||
        nextBehaviorDraft.parallelism.trim().length > 0,
    );
    setIsAvatarUploadPending(false);
    setImportErrorMessage(null);
    setIsImportingUpdate(false);
  }, [initialValues, open]);

  React.useEffect(() => {
    if (
      !open ||
      !initialValues ||
      "id" in initialValues ||
      initialValues.runtime?.trim() ||
      runtimesLoading ||
      runtime.trim().length > 0 ||
      defaultRuntime === null
    ) {
      return;
    }

    setRuntime(defaultRuntime.id);
  }, [defaultRuntime, initialValues, open, runtime, runtimesLoading]);

  const isWindowFileDragOver = useWindowFileDragOver(
    open && canImportPersonaUpdate,
  );

  React.useEffect(() => {
    if (!open || !importErrorMessage) {
      return;
    }
    const timeout = window.setTimeout(() => {
      setImportErrorMessage(null);
    }, IMPORT_ERROR_VISIBILITY_MS);
    return () => {
      window.clearTimeout(timeout);
    };
  }, [importErrorMessage, open]);

  async function handleImportUpdateSelection(
    fileBytes: number[],
    fileName: string,
  ) {
    if (!editPersonaId || !onImportUpdateFile) {
      return;
    }

    setImportErrorMessage(null);
    setIsImportingUpdate(true);
    try {
      await onImportUpdateFile(editPersonaId, fileBytes, fileName);
    } catch (error) {
      setImportErrorMessage(
        getImportErrorLabel(error instanceof Error ? error.message : null),
      );
    } finally {
      setIsImportingUpdate(false);
    }
  }

  const {
    fileInputRef: importFileInputRef,
    isDragOver: isImportDragOver,
    dropHandlers: importDropHandlers,
    handleFileChange: handleImportFileChange,
    openFilePicker: openImportFilePicker,
  } = useFileImportZone({
    onImportFile: (fileBytes, fileName) => {
      void handleImportUpdateSelection(fileBytes, fileName);
    },
  });

  function handleOpenChange(next: boolean) {
    if (!next) {
      setDisplayName("");
      setAvatarUrl("");
      setSystemPrompt("");
      setRuntime("");
      setModel("");
      setIsCustomModelEditing(false);
      setProvider("");
      setIsCustomProviderEditing(false);
      setNamePoolText("");
      setEnvVars({});
      setBehaviorDraft(emptyPersonaBehaviorDraft);
      behaviorSeedRef.current = emptyPersonaBehaviorDraft;
      setShowAdvancedFields(false);
      setIsAvatarUploadPending(false);
      setImportErrorMessage(null);
      setIsImportingUpdate(false);
    }

    onOpenChange(next);
  }

  async function handleSubmit() {
    if (
      !initialValues ||
      !canSubmitPersonaDialog({ displayName, isPending }) ||
      isAvatarUploadPending
    ) {
      return;
    }

    const trimmedRuntime = runtime.trim();
    const previousRuntime = initialValues.runtime?.trim() ?? "";
    const modelProviderEditableWithoutRuntime =
      initialModelProviderEditableWithoutRuntime && trimmedRuntime.length === 0;
    const llmProviderVisibleForSubmit =
      (trimmedRuntime.length > 0 &&
        runtimeSupportsLlmProviderSelection(trimmedRuntime)) ||
      modelProviderEditableWithoutRuntime;
    const shouldPreserveHiddenModelProvider =
      "id" in initialValues &&
      previousRuntime.length === 0 &&
      trimmedRuntime.length === 0 &&
      !modelProviderEditableWithoutRuntime;
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
      runtime: trimmedRuntime || undefined,
      model:
        trimmedRuntime || modelProviderEditableWithoutRuntime
          ? model.trim() || undefined
          : shouldPreserveHiddenModelProvider
            ? initialValues.model
            : undefined,
      provider: llmProviderVisibleForSubmit
        ? provider.trim() || undefined
        : shouldPreserveHiddenModelProvider
          ? initialValues.provider
          : undefined,
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

  const importButtonTone = getImportButtonTone({
    isWindowFileDragOver,
    isImportDragOver,
    importErrorMessage,
  });
  const importButtonLabel = getImportButtonLabel({
    isWindowFileDragOver,
    isImportDragOver,
    importErrorMessage,
  });

  const selectedRuntime = runtimes.find((p) => p.id === runtime);
  const blankRuntimeModelProviderEditable =
    initialModelProviderEditableWithoutRuntime && runtime.trim().length === 0;
  const runtimeCanChooseLlmProvider =
    runtimeSupportsLlmProviderSelection(runtime) ||
    blankRuntimeModelProviderEditable;
  const llmProviderFieldVisible =
    (runtime.trim().length > 0 && runtimeCanChooseLlmProvider) ||
    blankRuntimeModelProviderEditable;
  const providerForModelScope = llmProviderFieldVisible ? provider : "";
  const trimmedProvider = provider.trim();
  const providerApiKeyConfig =
    llmProviderFieldVisible && !isCustomProviderEditing
      ? getProviderApiKeyConfig(trimmedProvider)
      : null;
  const providerApiKeyValue = providerApiKeyConfig
    ? (envVars[providerApiKeyConfig.envVar] ?? "")
    : "";
  const providerApiKeyFieldVisible =
    llmProviderFieldVisible && providerApiKeyConfig !== null;
  // Required credential env keys for this runtime + provider combination.
  // Used to show required markers on the LLM provider label and amber
  // locked rows in the env vars editor.
  // File-layer config for the selected runtime (e.g. goose config.yaml).
  // Used to silence requirements already satisfied there.
  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(runtime, {
    enabled: open,
  });
  const { data: bakedEnvKeys } = useBakedBuildEnvKeysQuery({ enabled: open });
  const localModeGate = React.useMemo(
    () =>
      computeLocalModeGate({
        bakedEnvKeys,
        envVars,
        globalEnvVars: globalConfig.env_vars,
        globalProvider: globalConfig.provider ?? "",
        globalModel: globalConfig.model ?? "",
        isProviderMode: false,
        model,
        provider: trimmedProvider,
        runtimeId: runtime,
        runtimeFileConfig,
        useMesh: false,
      }),
    [
      bakedEnvKeys,
      envVars,
      globalConfig.env_vars,
      globalConfig.provider,
      globalConfig.model,
      model,
      trimmedProvider,
      runtime,
      runtimeFileConfig,
    ],
  );
  // requiredEnvKeys: the gate already handles baked-, global-, and file-
  // satisfied keys so no further filtering is needed.
  const { requiredEnvKeys } = localModeGate;
  // Provider required-ness is a static property of the runtime — it does not
  // change based on whether the field is currently filled. Using the dynamic
  // missingNormalizedFields check would flip the asterisk off once a value is
  // selected, which is incoherent (required means required, not "required until
  // satisfied"). runtimeSupportsLlmProviderSelection is the authoritative gate.
  const providerIsRequired = runtimeSupportsLlmProviderSelection(runtime);
  const modelFieldVisible =
    runtime.trim().length > 0 || blankRuntimeModelProviderEditable;
  const isExplicitModelRequired =
    modelFieldVisible && providerRequiresExplicitModel(providerForModelScope);
  const isCreateMode = Boolean(initialValues && !("id" in initialValues));
  const selectedRuntimeIsAvailable =
    runtime.trim().length === 0 ||
    selectedRuntime?.availability === "available";
  const canSubmit =
    canSubmitPersonaDialog({ displayName, isPending }) &&
    (!isCreateMode || runtime.trim().length > 0) &&
    (!isCreateMode || selectedRuntimeIsAvailable) &&
    (!isCreateMode || !createSubmitBlocked) &&
    // Crash-loop guard, create AND edit: an empty allowlist would crash
    // every instance minted from this definition at startup.
    personaBehaviorDraftValid(behaviorDraft) &&
    (!isExplicitModelRequired || model.trim().length > 0) &&
    // Block save when the LLM provider field is visible but no effective
    // provider exists — neither a per-agent value nor a global fallback.
    // When a global provider is set, an empty per-agent provider is valid
    // (inherits the global). Only block when there is genuinely nothing.
    !(
      llmProviderFieldVisible &&
      !provider.trim() &&
      !(globalConfig.provider ?? "").trim()
    ) &&
    !isAvatarUploadPending;

  // Auto-expand the Advanced section once per dialog-open cycle when required
  // env keys are present, so the user sees a clear signal that action is
  // needed (e.g. provider API key required). Does not re-open if the user
  // manually collapses the section afterward.
  const hasAutoOpenedAdvancedRef = React.useRef(false);
  React.useEffect(() => {
    if (!open) {
      hasAutoOpenedAdvancedRef.current = false;
      return;
    }
    if (requiredEnvKeys.length > 0 && !hasAutoOpenedAdvancedRef.current) {
      hasAutoOpenedAdvancedRef.current = true;
      setShowAdvancedFields(true);
    }
  }, [open, requiredEnvKeys.length]);

  // Auto-expand Advanced once per open when the selected runtime is buzz-agent
  // so the model-tuning knobs are immediately reachable — mirrors the agent
  // instance dialogs' behavior.
  const hasAutoOpenedForBuzzAgentRef = React.useRef(false);
  React.useEffect(() => {
    if (!open) {
      hasAutoOpenedForBuzzAgentRef.current = false;
      return;
    }
    if (isBuzzAgentRuntime(runtime) && !hasAutoOpenedForBuzzAgentRef.current) {
      hasAutoOpenedForBuzzAgentRef.current = true;
      setShowAdvancedFields(true);
    }
  }, [open, runtime]);
  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars,
    isCustomProviderEditing,
    modelFieldVisible,
    open,
    provider: providerForModelScope,
    selectedRuntime,
  });
  const staticModelOptions = getPersonaModelOptions(
    runtime,
    providerForModelScope,
  );
  const runtimeModelOptions = getRuntimePersonaModelOptions(runtime);
  const modelOptions = discoveredModelOptions ?? staticModelOptions;
  const isModelCustom = !hasPersonaModelOption(
    discoveredModelOptions ?? runtimeModelOptions,
    model,
  );
  const modelSelectValue = getModelSelectValue({
    isCustomModelEditing,
    isModelCustom,
    model,
  });
  const showCustomModelInput =
    modelFieldVisible && (isCustomModelEditing || isModelCustom);
  const providerOptions = getPersonaProviderOptions(
    providerForModelScope,
    runtime,
    globalConfig.provider ?? "",
  );
  const defaultLlmProviderLabel = getDefaultLlmProviderLabel(
    runtime,
    globalConfig.provider ?? "",
  );
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : trimmedProvider || AUTO_PROVIDER_DROPDOWN_VALUE;
  const showCustomProviderInput =
    llmProviderFieldVisible && isCustomProviderEditing;
  const advancedEnvVars = getAdvancedEnvVars(
    envVars,
    providerApiKeyConfig?.envVar ?? null,
  );
  const runtimeDropdownValue = runtime.trim() || NO_RUNTIME_DROPDOWN_VALUE;
  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(runtimes),
    [runtimes],
  );
  const blankRuntimeOptionLabel = runtimesLoading
    ? "Loading providers..."
    : isCreateMode
      ? "Choose a provider"
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
    ...providerOptions.map((option) => ({
      label: option.label,
      value: option.id || AUTO_PROVIDER_DROPDOWN_VALUE,
    })),
    { label: "Custom provider...", value: CUSTOM_PROVIDER_DROPDOWN_VALUE },
  ];
  const modelDropdownOptions: PersonaDropdownOption[] = [
    ...modelOptions.map((option) => ({
      label: option.label,
      value: option.id || AUTO_MODEL_DROPDOWN_VALUE,
    })),
    ...(modelDiscoveryLoading && discoveredModelOptions === null
      ? [
          {
            disabled: true,
            label: "Loading models...",
            value: MODEL_DISCOVERY_LOADING_VALUE,
          },
        ]
      : []),
    { label: "Custom model...", value: CUSTOM_MODEL_DROPDOWN_VALUE },
  ];
  const previewLabel = displayName.trim() || "Agent name";
  const previewAvatarUrl = avatarUrl.trim() || null;
  const runtimeWarning =
    selectedRuntime && selectedRuntime.availability !== "available" ? (
      <p className="text-xs text-warning">
        {selectedRuntime.availability === "adapter_missing"
          ? `${selectedRuntime.label} CLI is installed but the ACP adapter is missing.`
          : selectedRuntime.availability === "cli_missing"
            ? `${selectedRuntime.label} ACP adapter is installed but the CLI is missing.`
            : `${selectedRuntime.label} is not installed.`}{" "}
        Visit Settings &gt; Doctor to set it up.
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
        provider: providerForModelScope,
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
    providerForModelScope,
    runtime,
  ]);

  function handleProviderApiKeyChange(value: string) {
    if (!providerApiKeyConfig) {
      return;
    }

    setEnvVars((current) =>
      envVarsWithProviderApiKey(current, providerApiKeyConfig.envVar, value),
    );
  }

  function handleAdvancedEnvVarsChange(nextAdvancedEnvVars: EnvVarsValue) {
    setEnvVars((current) =>
      envVarsMergingAdvancedEdit(
        current,
        nextAdvancedEnvVars,
        providerApiKeyConfig?.envVar ?? null,
      ),
    );
  }

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
    applySelection(
      selectionOnProviderDropdownChange(selection, {
        runtime,
        nextValue,
        clearModelWhenApiKeyMissing: true,
      }),
    );
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
              {canImportPersonaUpdate ? (
                <>
                  <input
                    accept=".md,.json,.png,.zip"
                    className="hidden"
                    onChange={handleImportFileChange}
                    ref={importFileInputRef}
                    type="file"
                  />
                  <button
                    className={cn(
                      "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-xs font-medium transition-colors",
                      importButtonTone === "drag"
                        ? "border-dashed border-primary/70 bg-primary/10 text-primary"
                        : importButtonTone === "error"
                          ? "border-destructive/40 bg-destructive/10 text-destructive hover:bg-destructive/15"
                          : "border-border bg-background text-muted-foreground hover:bg-muted hover:text-foreground",
                    )}
                    disabled={isPending || isImportPending || isImportingUpdate}
                    type="button"
                    {...importDropHandlers}
                    onClick={openImportFilePicker}
                    title={
                      importButtonTone === "error"
                        ? importButtonLabel
                        : undefined
                    }
                  >
                    <Upload className="h-4 w-4" />
                    <span className="max-w-[16rem] truncate">
                      {importButtonLabel}
                    </span>
                    {isImportingUpdate ? (
                      <RefreshCw className="h-4 w-4 animate-spin" />
                    ) : null}
                  </button>
                </>
              ) : (
                createFooterSlot
              )}
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
                Agent instruction
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
                Agent runtime
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

            {llmProviderFieldVisible ? (
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
                  placeholder={defaultLlmProviderLabel}
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
                {providerApiKeyFieldVisible && providerApiKeyConfig ? (
                  <PersonaProviderApiKeyField
                    config={providerApiKeyConfig}
                    disabled={isPending}
                    onChange={handleProviderApiKeyChange}
                    value={providerApiKeyValue}
                  />
                ) : null}
              </div>
            ) : null}

            <AnimatePresence initial={false}>
              {modelFieldVisible ? (
                <PersonaModelField
                  disabled={isPending}
                  isExplicitModelRequired={isExplicitModelRequired}
                  model={model}
                  modelDiscoveryStatus={modelDiscoveryStatus}
                  modelDropdownOptions={modelDropdownOptions}
                  modelSelectValue={modelSelectValue}
                  onCustomModelChange={setModel}
                  onModelValueChange={handleModelDropdownChange}
                  showCustomModelInput={showCustomModelInput}
                  transition={advancedFieldsTransition}
                />
              ) : null}
            </AnimatePresence>

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
                      envVars={advancedEnvVars}
                      fileSatisfiedEnvKeys={localModeGate.fileSatisfiedEnvKeys}
                      inheritedEnvVars={globalConfig.env_vars}
                      modelTuningRuntimeId={runtime}
                      namePoolText={namePoolText}
                      onBehaviorDraftChange={setBehaviorDraft}
                      onEnvVarsChange={handleAdvancedEnvVarsChange}
                      onNamePoolTextChange={setNamePoolText}
                      requiredEnvKeys={requiredEnvKeys.filter(
                        (k) => k !== (providerApiKeyConfig?.envVar ?? null),
                      )}
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
