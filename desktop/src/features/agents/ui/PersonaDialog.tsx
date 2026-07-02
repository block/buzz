import * as React from "react";
import { ChevronDown, RefreshCw, Upload } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
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
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
} from "./personaDialogPickers";
import { shouldClearModelForRuntimeChange } from "./personaRuntimeModel";
import { RequiredFieldLabel } from "./personaProviderModelFields";
import {
  MODEL_DISCOVERY_LOADING_VALUE,
  usePersonaModelDiscovery,
} from "./usePersonaModelDiscovery";
import { useRuntimeFileConfigQuery } from "../hooks";

type PersonaDialogProps = {
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
  onSubmit: (input: CreatePersonaInput | UpdatePersonaInput) => Promise<void>;
  onImportUpdateFile?: (
    personaId: string,
    fileBytes: number[],
    fileName: string,
  ) => Promise<void>;
};

const ADVANCED_FIELDS_MOTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function PersonaDialog({
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
}: PersonaDialogProps) {
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
  const [showAdvancedFields, setShowAdvancedFields] = React.useState(false);
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const [isImportingUpdate, setIsImportingUpdate] = React.useState(false);
  const [importErrorMessage, setImportErrorMessage] = React.useState<
    string | null
  >(null);
  const [isWindowFileDragOver, setIsWindowFileDragOver] = React.useState(false);
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
    setNamePoolText(nextNamePoolText);
    setEnvVars(nextEnvVars);
    setShowAdvancedFields(
      nextNamePoolText.trim().length > 0 ||
        hasAdvancedEnvVars(nextEnvVars, managedApiKeyEnvVar),
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

  React.useEffect(() => {
    if (!open || !canImportPersonaUpdate) {
      setIsWindowFileDragOver(false);
      return;
    }

    let dragDepth = 0;

    function isFileDrag(event: DragEvent): boolean {
      return Array.from(event.dataTransfer?.types ?? []).includes("Files");
    }

    function handleWindowDragEnter(event: DragEvent) {
      if (!isFileDrag(event)) {
        return;
      }
      dragDepth += 1;
      setIsWindowFileDragOver(true);
    }

    function handleWindowDragOver(event: DragEvent) {
      if (!isFileDrag(event)) {
        return;
      }
      event.preventDefault();
      if (event.dataTransfer) {
        event.dataTransfer.dropEffect = "copy";
      }
      setIsWindowFileDragOver(true);
    }

    function handleWindowDragLeave(event: DragEvent) {
      if (!isFileDrag(event)) {
        return;
      }
      dragDepth = Math.max(0, dragDepth - 1);
      if (dragDepth === 0) {
        setIsWindowFileDragOver(false);
      }
    }

    function handleWindowDrop(event: DragEvent) {
      if (!isFileDrag(event)) {
        return;
      }
      event.preventDefault();
      dragDepth = 0;
      setIsWindowFileDragOver(false);
    }

    window.addEventListener("dragenter", handleWindowDragEnter);
    window.addEventListener("dragover", handleWindowDragOver);
    window.addEventListener("dragleave", handleWindowDragLeave);
    window.addEventListener("drop", handleWindowDrop);

    return () => {
      window.removeEventListener("dragenter", handleWindowDragEnter);
      window.removeEventListener("dragover", handleWindowDragOver);
      window.removeEventListener("dragleave", handleWindowDragLeave);
      window.removeEventListener("drop", handleWindowDrop);
    };
  }, [canImportPersonaUpdate, open]);

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
      setShowAdvancedFields(false);
      setIsAvatarUploadPending(false);
      setImportErrorMessage(null);
      setIsImportingUpdate(false);
      setIsWindowFileDragOver(false);
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
  const localModeGate = computeLocalModeGate({
    envVars,
    isProviderMode: false,
    model,
    provider: trimmedProvider,
    runtimeId: runtime,
    runtimeFileConfig,
    useMesh: false,
  });
  // Required keys for EnvVarsEditor amber rows: exclude file-satisfied keys
  // so they render in the "Set in goose config" row instead.
  const requiredEnvKeys = requiredCredentialEnvKeys(
    runtime,
    trimmedProvider,
  ).filter((key) => !localModeGate.fileSatisfiedEnvKeys.includes(key));
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
    (!isExplicitModelRequired || model.trim().length > 0) &&
    !isAvatarUploadPending;
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
  );
  const defaultLlmProviderLabel = getDefaultLlmProviderLabel(runtime);
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

  function updateProviderApiKey(envKey: string, value: string) {
    setEnvVars((current) => {
      if ((current[envKey] ?? "") === value) {
        return current;
      }

      const next = { ...current };
      if (value.length > 0) {
        next[envKey] = value;
      } else {
        delete next[envKey];
      }
      return next;
    });
  }

  function removeEnvVar(envKey: string) {
    setEnvVars((current) => {
      if (!(envKey in current)) {
        return current;
      }

      const next = { ...current };
      delete next[envKey];
      return next;
    });
  }

  function clearManagedProviderApiKeyWhenLeaving(
    previousProvider: string,
    nextProvider: string,
  ) {
    const previousEnvVar = getProviderApiKeyEnvVar(previousProvider);
    const nextEnvVar = getProviderApiKeyEnvVar(nextProvider);
    if (previousEnvVar && previousEnvVar !== nextEnvVar) {
      removeEnvVar(previousEnvVar);
    }
  }

  function handleProviderApiKeyChange(value: string) {
    if (!providerApiKeyConfig) {
      return;
    }

    updateProviderApiKey(providerApiKeyConfig.envVar, value);
  }

  function handleAdvancedEnvVarsChange(nextAdvancedEnvVars: EnvVarsValue) {
    setEnvVars((current) => {
      const managedEnvVar = providerApiKeyConfig?.envVar ?? null;
      if (!managedEnvVar || !(managedEnvVar in current)) {
        return nextAdvancedEnvVars;
      }

      return {
        ...nextAdvancedEnvVars,
        [managedEnvVar]: current[managedEnvVar],
      };
    });
  }

  function handleRuntimeDropdownChange(nextValue: string) {
    const nextRuntime =
      nextValue === NO_RUNTIME_DROPDOWN_VALUE ? "" : nextValue;
    const previousRuntime = runtime;
    const nextRuntimeCanChooseLlmProvider =
      nextRuntime.trim().length > 0 &&
      runtimeSupportsLlmProviderSelection(nextRuntime);
    setRuntime(nextRuntime);
    if (
      shouldClearModelForRuntimeChange(previousRuntime, nextRuntime) ||
      shouldClearKnownModelForSelectionScope({
        model,
        provider,
        runtime: nextRuntime,
      })
    ) {
      setModel("");
      setIsCustomModelEditing(false);
    }
    if (!nextRuntimeCanChooseLlmProvider) {
      clearManagedProviderApiKeyWhenLeaving(provider, "");
      setIsCustomModelEditing(false);
      setIsCustomProviderEditing(false);
      setProvider("");
    }
  }

  function handleProviderDropdownChange(nextValue: string) {
    if (nextValue === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
      clearManagedProviderApiKeyWhenLeaving(provider, "");
      setIsCustomProviderEditing(true);
      setProvider("");
      return;
    }

    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
    clearManagedProviderApiKeyWhenLeaving(provider, nextProvider);
    setIsCustomProviderEditing(false);
    setProvider(nextProvider);
    const requiredEnvVar = getProviderApiKeyEnvVar(nextProvider);
    if (requiredEnvVar && !envVars[requiredEnvVar]?.trim()) {
      setModel("");
      setIsCustomModelEditing(false);
    }
    if (
      !isCustomModelEditing &&
      shouldClearKnownModelForSelectionScope({
        model,
        provider: nextProvider,
        runtime,
      })
    ) {
      setModel("");
      setIsCustomModelEditing(false);
    }
  }

  function handleModelDropdownChange(nextValue: string) {
    if (nextValue === CUSTOM_MODEL_DROPDOWN_VALUE) {
      setIsCustomModelEditing(true);
      if (!isModelCustom) {
        setModel("");
      }
      return;
    }

    setIsCustomModelEditing(false);
    setModel(nextValue === AUTO_MODEL_DROPDOWN_VALUE ? "" : nextValue);
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
                      disabled={isPending}
                      envVars={advancedEnvVars}
                      fileSatisfiedEnvKeys={localModeGate.fileSatisfiedEnvKeys}
                      namePoolText={namePoolText}
                      onEnvVarsChange={handleAdvancedEnvVarsChange}
                      onNamePoolTextChange={setNamePoolText}
                      requiredEnvKeys={requiredEnvKeys}
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
