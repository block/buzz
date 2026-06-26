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
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
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
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getDefaultPersonaRuntime,
  getModelSelectValue,
  getPersonaModelOptions,
  getPersonaProviderOptions,
  getRuntimePersonaModelOptions,
  hasPersonaModelOption,
  NO_RUNTIME_DROPDOWN_VALUE,
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  shouldClearKnownModelForSelectionScope,
} from "./personaDialogPickers";
import { shouldClearModelForRuntimeChange } from "./personaRuntimeModel";

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

const PERSONA_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";
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
    setNamePoolText(nextNamePoolText);
    setEnvVars(nextEnvVars);
    setShowAdvancedFields(
      nextNamePoolText.trim().length > 0 || Object.keys(nextEnvVars).length > 0,
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
    const shouldPreserveHiddenModelProvider =
      "id" in initialValues &&
      previousRuntime.length === 0 &&
      trimmedRuntime.length === 0;
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
      systemPrompt: systemPrompt.trim(),
      runtime: trimmedRuntime || undefined,
      model: trimmedRuntime
        ? model.trim() || undefined
        : shouldPreserveHiddenModelProvider
          ? initialValues.model
          : undefined,
      provider: trimmedRuntime
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
  const llmProviderFieldVisible = runtime.trim().length > 0;
  const modelFieldVisible =
    llmProviderFieldVisible &&
    (isCustomProviderEditing ||
      provider.trim().length > 0 ||
      model.trim().length > 0 ||
      isCustomModelEditing);
  const isCreateMode = Boolean(initialValues && !("id" in initialValues));
  const selectedRuntimeIsAvailable =
    runtime.trim().length === 0 ||
    selectedRuntime?.availability === "available";
  const canSubmit =
    canSubmitPersonaDialog({ displayName, isPending }) &&
    (!isCreateMode || runtime.trim().length > 0) &&
    (!isCreateMode || selectedRuntimeIsAvailable) &&
    !isAvatarUploadPending;
  const modelOptions = getPersonaModelOptions(runtime, provider);
  const runtimeModelOptions = getRuntimePersonaModelOptions(runtime);
  const isModelCustom = !hasPersonaModelOption(runtimeModelOptions, model);
  const modelSelectValue = getModelSelectValue({
    isCustomModelEditing,
    isModelCustom,
    model,
  });
  const showCustomModelInput =
    modelFieldVisible && (isCustomModelEditing || isModelCustom);
  const providerOptions = getPersonaProviderOptions(provider);
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : provider.trim() || AUTO_PROVIDER_DROPDOWN_VALUE;
  const showCustomProviderInput =
    llmProviderFieldVisible && isCustomProviderEditing;
  const runtimeDropdownValue = runtime.trim() || NO_RUNTIME_DROPDOWN_VALUE;
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
    ...runtimes.map((candidate) => ({
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
      !shouldClearKnownModelForSelectionScope({ model, provider, runtime })
    ) {
      return;
    }

    setModel("");
    setIsCustomModelEditing(false);
  }, [isCustomModelEditing, model, modelFieldVisible, open, provider, runtime]);

  function handleRuntimeDropdownChange(nextValue: string) {
    const nextRuntime =
      nextValue === NO_RUNTIME_DROPDOWN_VALUE ? "" : nextValue;
    const previousRuntime = runtime;
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
    if (nextRuntime.trim().length === 0) {
      setIsCustomModelEditing(false);
      setIsCustomProviderEditing(false);
      setProvider("");
    }
  }

  function handleProviderDropdownChange(nextValue: string) {
    if (nextValue === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
      setIsCustomProviderEditing(true);
      setProvider("");
      return;
    }

    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
    setIsCustomProviderEditing(false);
    setProvider(nextProvider);
    if (nextProvider.trim().length === 0) {
      setModel("");
      setIsCustomModelEditing(false);
      return;
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
                Provider
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
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor="persona-llm-provider"
                >
                  LLM provider
                  <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
                </label>
                <PersonaDropdownField
                  disabled={isPending}
                  id="persona-llm-provider"
                  onValueChange={handleProviderDropdownChange}
                  options={providerDropdownOptions}
                  placeholder="Auto (default)"
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

            <AnimatePresence initial={false}>
              {modelFieldVisible ? (
                <motion.div
                  animate={{ height: "auto", opacity: 1, scale: 1 }}
                  className="origin-top overflow-hidden"
                  exit={{ height: 0, opacity: 0, scale: 0.98 }}
                  initial={{ height: 0, opacity: 0, scale: 0.98 }}
                  key="persona-model-field"
                  transition={advancedFieldsTransition}
                >
                  <div className="space-y-1.5">
                    <label
                      className="text-sm font-medium text-foreground"
                      htmlFor="persona-model"
                    >
                      Model
                      <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                        Optional
                      </span>
                    </label>
                    <PersonaDropdownField
                      disabled={isPending}
                      id="persona-model"
                      onValueChange={handleModelDropdownChange}
                      options={modelDropdownOptions}
                      placeholder="Auto (default)"
                      value={modelSelectValue}
                    />
                    {showCustomModelInput ? (
                      <div
                        className={cn(
                          "mt-2 flex min-h-11 items-center px-3",
                          PERSONA_FIELD_SHELL_CLASS,
                        )}
                      >
                        <Input
                          aria-label="Custom model ID"
                          autoCorrect="off"
                          className={cn(
                            "h-8 px-0 py-0 leading-6",
                            PERSONA_FIELD_CONTROL_CLASS,
                          )}
                          disabled={isPending}
                          id="persona-custom-model"
                          onChange={(event) => setModel(event.target.value)}
                          placeholder="Custom model ID"
                          value={model}
                        />
                      </div>
                    ) : null}
                  </div>
                </motion.div>
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
                    <div className="space-y-5 pt-2">
                      <div className="space-y-1.5">
                        <label
                          className="text-sm font-medium text-foreground"
                          htmlFor="persona-name-pool"
                        >
                          Instance name pool
                          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                            Optional
                          </span>
                        </label>
                        <div
                          className={cn(
                            "flex min-h-11 items-center px-3",
                            PERSONA_FIELD_SHELL_CLASS,
                          )}
                        >
                          <Input
                            autoCapitalize="words"
                            autoCorrect="off"
                            className={cn(
                              "h-8 px-0 py-0 leading-6",
                              PERSONA_FIELD_CONTROL_CLASS,
                            )}
                            disabled={isPending}
                            id="persona-name-pool"
                            onChange={(event) =>
                              setNamePoolText(event.target.value)
                            }
                            placeholder="Birch, Compass, Ridge, Thistle"
                            spellCheck={false}
                            value={namePoolText}
                          />
                        </div>
                      </div>

                      <EnvVarsEditor
                        disabled={isPending}
                        onChange={setEnvVars}
                        value={envVars}
                      />
                    </div>
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
