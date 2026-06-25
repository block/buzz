import * as React from "react";
import { RefreshCw, Upload } from "lucide-react";

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

const PERSONA_FIELD_SHELL_CLASS =
  "rounded-xl border border-input bg-muted/40 transition-colors duration-150 ease-out hover:border-muted-foreground/40 focus-within:border-muted-foreground/50";
const PERSONA_FIELD_CONTROL_CLASS =
  "border-0 bg-transparent text-muted-foreground shadow-none outline-none ring-0 transition-colors duration-150 ease-out placeholder:text-muted-foreground/55 focus:bg-transparent focus:text-muted-foreground focus:outline-hidden focus-visible:ring-0";
const PERSONA_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";
const AUTO_MODEL_DROPDOWN_VALUE = "__auto_model__";
const CUSTOM_MODEL_DROPDOWN_VALUE = "__custom_model__";
const AUTO_PROVIDER_DROPDOWN_VALUE = "__auto_provider__";
const CUSTOM_PROVIDER_DROPDOWN_VALUE = "__custom_provider__";

type PersonaModelOption = {
  id: string;
  label: string;
};

const AUTO_MODEL_OPTION: PersonaModelOption = {
  id: "",
  label: "Auto (provider default)",
};

const PERSONA_LLM_PROVIDER_OPTIONS: readonly PersonaModelOption[] = [
  { id: "", label: "Auto (runtime default)" },
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "openai-compat", label: "OpenAI-compatible" },
  { id: "databricks", label: "Databricks" },
];

const PERSONA_MODEL_OPTIONS_BY_RUNTIME: Record<
  string,
  readonly PersonaModelOption[]
> = {
  goose: [
    AUTO_MODEL_OPTION,
    { id: "goose-claude-4-6-opus", label: "Claude Opus 4.6" },
    { id: "goose-claude-4-6-sonnet", label: "Claude Sonnet 4.6" },
    { id: "gpt-5", label: "GPT-5" },
    { id: "gpt-5-mini", label: "GPT-5 mini" },
    { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
    { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
  ],
  "buzz-agent": [
    AUTO_MODEL_OPTION,
    { id: "goose-claude-4-6-opus", label: "Claude Opus 4.6" },
    { id: "goose-claude-4-6-sonnet", label: "Claude Sonnet 4.6" },
    { id: "gpt-5", label: "GPT-5" },
    { id: "gpt-5-mini", label: "GPT-5 mini" },
    { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
    { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
  ],
  claude: [AUTO_MODEL_OPTION],
  codex: [AUTO_MODEL_OPTION],
};

function getPersonaModelOptions(
  runtimeId: string,
): readonly PersonaModelOption[] {
  return PERSONA_MODEL_OPTIONS_BY_RUNTIME[runtimeId] ?? [AUTO_MODEL_OPTION];
}

function hasPersonaModelOption(
  options: readonly PersonaModelOption[],
  modelId: string,
) {
  const trimmedModel = modelId.trim();
  return (
    trimmedModel.length === 0 ||
    options.some((option) => option.id === trimmedModel)
  );
}

function getModelSelectValue({
  isCustomModelEditing,
  isModelCustom,
  model,
}: {
  isCustomModelEditing: boolean;
  isModelCustom: boolean;
  model: string;
}) {
  if (isCustomModelEditing || isModelCustom) {
    return CUSTOM_MODEL_DROPDOWN_VALUE;
  }

  return model.trim() || AUTO_MODEL_DROPDOWN_VALUE;
}

function getPersonaProviderOptions(
  currentProvider: string,
): readonly PersonaModelOption[] {
  const trimmedProvider = currentProvider.trim();
  if (
    trimmedProvider.length === 0 ||
    PERSONA_LLM_PROVIDER_OPTIONS.some((option) => option.id === trimmedProvider)
  ) {
    return PERSONA_LLM_PROVIDER_OPTIONS;
  }

  return [
    ...PERSONA_LLM_PROVIDER_OPTIONS,
    { id: trimmedProvider, label: `${trimmedProvider} (current)` },
  ];
}

function formatRuntimeOptionLabel(runtime: AcpRuntimeCatalogEntry) {
  const suffix =
    runtime.availability === "adapter_missing"
      ? " (adapter missing)"
      : runtime.availability === "cli_missing"
        ? " (CLI missing)"
        : runtime.availability === "not_installed"
          ? " (not installed)"
          : "";
  return `${runtime.label}${suffix}`;
}

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
    setNamePoolText(
      "namePool" in initialValues
        ? formatPersonaNamePoolText(initialValues.namePool)
        : "",
    );
    setEnvVars("envVars" in initialValues ? (initialValues.envVars ?? {}) : {});
    setIsAvatarUploadPending(false);
    setImportErrorMessage(null);
    setIsImportingUpdate(false);
  }, [initialValues, open]);

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
  const modelFieldVisible = runtime.trim().length > 0;
  const isCreateMode = Boolean(initialValues && !("id" in initialValues));
  const selectedRuntimeIsAvailable =
    runtime.trim().length === 0 ||
    selectedRuntime?.availability === "available";
  const canSubmit =
    canSubmitPersonaDialog({ displayName, isPending }) &&
    (!isCreateMode || runtime.trim().length > 0) &&
    (!isCreateMode || selectedRuntimeIsAvailable) &&
    !isAvatarUploadPending;
  const modelOptions = getPersonaModelOptions(runtime);
  const isModelCustom = !hasPersonaModelOption(modelOptions, model);
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
  const showCustomProviderInput = modelFieldVisible && isCustomProviderEditing;
  const blankRuntimeOptionLabel =
    isCreateMode && runtimesLoading
      ? "Loading providers..."
      : isCreateMode
        ? "Choose a provider"
        : "No preference (use app default)";
  const selectedModelLabel =
    modelOptions.find((option) => option.id === model.trim())?.label ??
    AUTO_MODEL_OPTION.label;
  const selectedProviderLabel =
    providerOptions.find((option) => option.id === provider)?.label ??
    (provider.trim()
      ? `${provider.trim()} (current)`
      : "Auto (runtime default)");
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
              <div className={PERSONA_FIELD_SHELL_CLASS}>
                <select
                  className={cn(
                    "h-11 w-full appearance-none px-3 py-2",
                    PERSONA_FIELD_CONTROL_CLASS,
                  )}
                  disabled={isPending || runtimesLoading}
                  id="persona-runtime"
                  onChange={(event) => {
                    const nextRuntime = event.target.value;
                    const previousRuntime = runtime;
                    setRuntime(nextRuntime);
                    if (
                      shouldClearModelForRuntimeChange(
                        previousRuntime,
                        nextRuntime,
                      )
                    ) {
                      setModel("");
                      setIsCustomModelEditing(false);
                    }
                    if (nextRuntime.trim().length === 0) {
                      setIsCustomModelEditing(false);
                      setIsCustomProviderEditing(false);
                      setProvider("");
                    }
                  }}
                  value={runtime}
                >
                  <option disabled={isCreateMode} value="">
                    {blankRuntimeOptionLabel}
                  </option>
                  {runtimes.map((candidate) => (
                    <option
                      disabled={
                        isCreateMode && candidate.availability !== "available"
                      }
                      key={candidate.id}
                      value={candidate.id}
                    >
                      {formatRuntimeOptionLabel(candidate)}
                    </option>
                  ))}
                </select>
              </div>
              {runtimeWarning}
            </div>

            <div
              aria-hidden={!modelFieldVisible}
              className={cn(
                "grid overflow-hidden transition-[grid-template-rows,opacity] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
                modelFieldVisible
                  ? "grid-rows-[1fr] opacity-100"
                  : "grid-rows-[0fr] opacity-0",
              )}
            >
              <div className="min-h-0 overflow-hidden">
                <div
                  className={cn(
                    "space-y-1.5 transition-[transform,opacity] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
                    modelFieldVisible
                      ? "translate-y-0 opacity-100"
                      : "-translate-y-1 opacity-0",
                  )}
                >
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="persona-model"
                  >
                    Model
                    <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                      Optional
                    </span>
                  </label>
                  <div className={PERSONA_FIELD_SHELL_CLASS}>
                    <select
                      className={cn(
                        "h-11 w-full appearance-none px-3 py-2",
                        PERSONA_FIELD_CONTROL_CLASS,
                      )}
                      disabled={isPending || !modelFieldVisible}
                      id="persona-model"
                      onChange={(event) => {
                        const nextModel = event.target.value;
                        if (nextModel === CUSTOM_MODEL_DROPDOWN_VALUE) {
                          setIsCustomModelEditing(true);
                          if (!isModelCustom) {
                            setModel("");
                          }
                          return;
                        }

                        setIsCustomModelEditing(false);
                        setModel(
                          nextModel === AUTO_MODEL_DROPDOWN_VALUE
                            ? ""
                            : nextModel,
                        );
                      }}
                      value={modelSelectValue}
                    >
                      <option value="" disabled>
                        {selectedModelLabel}
                      </option>
                      {modelOptions.map((option) => (
                        <option
                          key={option.id || AUTO_MODEL_DROPDOWN_VALUE}
                          value={option.id || AUTO_MODEL_DROPDOWN_VALUE}
                        >
                          {option.label}
                        </option>
                      ))}
                      <option value={CUSTOM_MODEL_DROPDOWN_VALUE}>
                        Custom model...
                      </option>
                    </select>
                  </div>
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
                        disabled={isPending || !modelFieldVisible}
                        id="persona-custom-model"
                        onChange={(event) => setModel(event.target.value)}
                        placeholder="Custom model ID"
                        value={model}
                      />
                    </div>
                  ) : null}
                </div>
              </div>
            </div>

            <div
              aria-hidden={!modelFieldVisible}
              className={cn(
                "grid overflow-hidden transition-[grid-template-rows,opacity] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
                modelFieldVisible
                  ? "grid-rows-[1fr] opacity-100"
                  : "grid-rows-[0fr] opacity-0",
              )}
            >
              <div className="min-h-0 overflow-hidden">
                <div
                  className={cn(
                    "space-y-1.5 transition-[transform,opacity] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
                    modelFieldVisible
                      ? "translate-y-0 opacity-100"
                      : "-translate-y-1 opacity-0",
                  )}
                >
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="persona-llm-provider"
                  >
                    LLM provider
                    <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                      Optional
                    </span>
                  </label>
                  <div className={PERSONA_FIELD_SHELL_CLASS}>
                    <select
                      className={cn(
                        "h-11 w-full appearance-none px-3 py-2",
                        PERSONA_FIELD_CONTROL_CLASS,
                      )}
                      disabled={isPending || !modelFieldVisible}
                      id="persona-llm-provider"
                      onChange={(event) => {
                        const nextProvider = event.target.value;
                        if (nextProvider === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
                          setIsCustomProviderEditing(true);
                          setProvider("");
                          return;
                        }

                        setIsCustomProviderEditing(false);
                        setProvider(
                          nextProvider === AUTO_PROVIDER_DROPDOWN_VALUE
                            ? ""
                            : nextProvider,
                        );
                      }}
                      value={providerSelectValue}
                    >
                      <option value="" disabled>
                        {selectedProviderLabel}
                      </option>
                      {providerOptions.map((option) => (
                        <option
                          key={option.id || AUTO_PROVIDER_DROPDOWN_VALUE}
                          value={option.id || AUTO_PROVIDER_DROPDOWN_VALUE}
                        >
                          {option.label}
                        </option>
                      ))}
                      <option value={CUSTOM_PROVIDER_DROPDOWN_VALUE}>
                        Custom provider...
                      </option>
                    </select>
                  </div>
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
                        disabled={isPending || !modelFieldVisible}
                        id="persona-custom-provider"
                        onChange={(event) => setProvider(event.target.value)}
                        placeholder="Custom provider ID"
                        value={provider}
                      />
                    </div>
                  ) : null}
                </div>
              </div>
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="persona-name-pool"
              >
                Instance name pool
                <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
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
                  onChange={(event) => setNamePoolText(event.target.value)}
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

            {error ? (
              <p className="text-sm text-destructive">{error.message}</p>
            ) : null}
          </div>
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
