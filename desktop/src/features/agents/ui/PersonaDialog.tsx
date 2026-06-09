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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  getImportButtonLabel,
  getImportButtonTone,
  getImportErrorLabel,
  IMPORT_ERROR_VISIBILITY_MS,
} from "./personaDialogImportState";

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
  const [namePoolText, setNamePoolText] = React.useState("");
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>({});
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
    setNamePoolText(
      ("namePool" in initialValues
        ? (initialValues as { namePool?: string[] }).namePool
        : undefined
      )?.join(", ") ?? "",
    );
    setEnvVars(initialValues.envVars ?? {});
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
      setNamePoolText("");
      setImportErrorMessage(null);
      setIsImportingUpdate(false);
      setIsWindowFileDragOver(false);
    }

    onOpenChange(next);
  }

  async function handleSubmit() {
    if (!initialValues) {
      return;
    }

    const namePool = namePoolText
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const baseInput = {
      displayName,
      avatarUrl: avatarUrl.trim() || undefined,
      systemPrompt,
      runtime: runtime.trim() || undefined,
      model: model.trim() || undefined,
      namePool: namePool.length > 0 ? namePool : undefined,
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
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-2xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>{title}</DialogTitle>
            {description.trim().length > 0 ? (
              <DialogDescription>{description}</DialogDescription>
            ) : null}
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="persona-display-name"
              >
                Display name
              </label>
              <Input
                autoCorrect="off"
                disabled={isPending}
                id="persona-display-name"
                onChange={(event) => setDisplayName(event.target.value)}
                placeholder="Researcher"
                value={displayName}
              />
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="persona-avatar-url"
              >
                Avatar URL
              </label>
              <Input
                autoCapitalize="none"
                autoCorrect="off"
                disabled={isPending}
                id="persona-avatar-url"
                onChange={(event) => setAvatarUrl(event.target.value)}
                placeholder="https://example.com/avatar.png"
                spellCheck={false}
                value={avatarUrl}
              />
              <p className="text-xs text-muted-foreground">
                Optional. Deployed agents fall back to the runtime avatar if
                this is blank.
              </p>
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="persona-system-prompt"
              >
                System prompt
              </label>
              <Textarea
                className="min-h-40"
                disabled={isPending}
                id="persona-system-prompt"
                onChange={(event) => setSystemPrompt(event.target.value)}
                placeholder="Describe what this persona should do."
                value={systemPrompt}
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-sm font-medium" htmlFor="persona-runtime">
                Preferred runtime
              </label>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                disabled={isPending || runtimesLoading}
                id="persona-runtime"
                onChange={(event) => setRuntime(event.target.value)}
                value={runtime}
              >
                <option value="">
                  {runtimesLoading
                    ? "Loading runtimes..."
                    : "No preference (use default)"}
                </option>
                {runtimes.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                    {p.availability === "adapter_missing"
                      ? " (adapter missing)"
                      : p.availability === "cli_missing"
                        ? " (CLI missing)"
                        : p.availability === "not_installed"
                          ? " (not installed)"
                          : ""}
                  </option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground">
                Optional. When deploying this persona, the selected runtime will
                be pre-selected. Unavailable runtimes can be installed from
                Settings &gt; Doctor.
              </p>
              {runtimeWarning}
            </div>

            <div className="space-y-1.5">
              <label className="text-sm font-medium" htmlFor="persona-model">
                Preferred model
              </label>
              <Input
                autoCapitalize="none"
                autoCorrect="off"
                disabled={isPending}
                id="persona-model"
                onChange={(event) => setModel(event.target.value)}
                placeholder="e.g. gpt-4o, claude-sonnet-4-20250514"
                spellCheck={false}
                value={model}
              />
              <p className="text-xs text-muted-foreground">
                Optional. Passed to the agent at creation time. Leave blank to
                use the runtime default.
              </p>
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="persona-name-pool"
              >
                Instance name pool
              </label>
              <Input
                autoCapitalize="none"
                autoCorrect="off"
                disabled={isPending}
                id="persona-name-pool"
                onChange={(event) => setNamePoolText(event.target.value)}
                placeholder="Birch, Compass, Ridge, Thistle, ..."
                spellCheck={false}
                value={namePoolText}
              />
              <p className="text-xs text-muted-foreground">
                Comma-separated names for bot copies. Each instance gets a
                random name from this pool. Leave empty to use generic defaults.
              </p>
            </div>

            <EnvVarsEditor
              disabled={isPending}
              helperText="Injected when agents created from this persona spawn. Per-agent overrides can replace these."
              onChange={setEnvVars}
              value={envVars}
            />

            {error ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {error.message}
              </p>
            ) : null}
          </div>

          <div className="flex shrink-0 items-center justify-between gap-3 border-t border-border/60 px-6 py-4">
            <div className="flex min-h-8 items-center">
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
                      "inline-flex h-8 items-center gap-2 rounded-md border px-3 text-xs font-medium transition-colors",
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
                    <Upload className="h-3.5 w-3.5" />
                    <span className="max-w-[16rem] truncate">
                      {importButtonLabel}
                    </span>
                    {isImportingUpdate ? (
                      <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                    ) : null}
                  </button>
                </>
              ) : null}
            </div>

            <div className="flex items-center gap-2">
              <Button
                onClick={() => handleOpenChange(false)}
                size="sm"
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
              <Button
                disabled={
                  displayName.trim().length === 0 ||
                  systemPrompt.trim().length === 0 ||
                  isPending
                }
                onClick={() => void handleSubmit()}
                size="sm"
                type="button"
              >
                {isPending ? "Saving..." : submitLabel}
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
