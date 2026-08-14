import * as React from "react";
import { AlertCircle, Upload } from "lucide-react";

import type {
  AgentSnapshotImportPreview,
  AgentSnapshotImportResult,
} from "@/features/agents/hooks";
import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
} from "@/shared/api/types";
import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { AgentTradingCardPreview } from "./AgentTradingCardPreview";
import { AgentDialog } from "./AgentDialog";

// ── Types ─────────────────────────────────────────────────────────────────────

type ImportPhase = "preview" | "confirming" | "result";

type AgentSnapshotImportDialogProps = {
  open: boolean;
  /** Original snapshot bytes. PNG imports use these as the exact card art
   * shown during review instead of reconstructing a second card. */
  fileBytes: number[];
  fileName: string;
  /** Preview data loaded by the caller before opening. */
  preview: AgentSnapshotImportPreview;
  /** True while the confirm mutation is in-flight. */
  isConfirming: boolean;
  /** Set when the confirm mutation has returned a result. */
  result: AgentSnapshotImportResult | null;
  /** Error from the confirm mutation, if any. */
  confirmError: string | null;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus: "loading" | "ready" | "error";
  onBeginSetup: () => void;
  /** Creates the imported agent after the reviewed setup form is submitted. */
  onConfirm: (input: CreatePersonaInput) => Promise<boolean>;
  onOpenChange: (open: boolean) => void;
};

// ── Component ─────────────────────────────────────────────────────────────────

export function AgentSnapshotImportDialog({
  open,
  fileBytes,
  fileName,
  preview,
  isConfirming,
  result,
  confirmError,
  runtimes,
  runtimeCatalogStatus,
  onBeginSetup,
  onConfirm,
  onOpenChange,
}: AgentSnapshotImportDialogProps) {
  const [showSetup, setShowSetup] = React.useState(false);
  const setupValues = React.useMemo(
    () => importSetupValues(preview),
    [preview],
  );
  const phase: ImportPhase =
    result !== null ? "result" : isConfirming ? "confirming" : "preview";
  const format = importSnapshotFormat(fileName);
  const memoryLevel = importSnapshotMemoryLevel(preview.memoryLevel);
  const cardImageUrl = useImportedCardImageUrl(fileBytes, format);
  const agentId =
    preview.avatarUrl ??
    `${preview.displayName}:${preview.model ?? ""}:${preview.runtime ?? ""}`;

  if (showSetup) {
    return (
      <AgentDialog
        description=""
        error={confirmError ? new Error(confirmError) : null}
        initialValues={setupValues}
        isPending={isConfirming}
        mode="definition-edit"
        onOpenChange={onOpenChange}
        onSubmit={async (input) => {
          if ("id" in input) return;
          const imported = await onConfirm(input);
          if (imported) onOpenChange(false);
        }}
        open={open}
        runtimes={runtimes}
        runtimeCatalogStatus={runtimeCatalogStatus}
        submitLabel="Add agent"
        title="Set up your imported agent"
      />
    );
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-lg"
        data-testid="agent-snapshot-import-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <DialogTitle className="truncate">
            {phase === "result"
              ? "Agent imported"
              : `Import ${preview.displayName}`}
          </DialogTitle>
        </DialogHeader>

        {phase === "result" && result !== null ? (
          <div className="flex flex-col gap-6 pt-2">
            <ResultBody result={result} confirmError={confirmError} />
            <div className="flex items-center justify-end pt-2">
              <DialogClose asChild>
                <Button size="sm" type="button" variant="ghost">
                  Close
                </Button>
              </DialogClose>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-6 pt-2">
            <div className="py-4" data-testid="agent-snapshot-import-card-area">
              <AgentTradingCardPreview
                agentAvatarUrl={preview.avatarUrl}
                agentId={agentId}
                agentName={preview.displayName}
                cardImageUrl={cardImageUrl}
                format={format}
                memoryLevel={memoryLevel}
              />
            </div>

            {confirmError ? (
              <p
                aria-live="polite"
                className="text-sm text-destructive"
                data-testid="agent-snapshot-import-error"
              >
                {confirmError}
              </p>
            ) : null}

            <div
              className="flex items-center justify-end gap-2 pt-2"
              data-testid="agent-snapshot-import-footer"
            >
              <DialogClose asChild>
                <Button
                  disabled={isConfirming}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Cancel
                </Button>
              </DialogClose>
              <Button
                data-testid="agent-snapshot-import-confirm"
                disabled={isConfirming}
                onClick={() => {
                  onBeginSetup();
                  setShowSetup(true);
                }}
                size="sm"
                type="button"
              >
                <Upload className="h-4 w-4" />
                {isConfirming ? "Importing…" : "Import"}
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function useImportedCardImageUrl(
  fileBytes: number[],
  format: SnapshotFormat,
): string | undefined {
  return React.useMemo(() => {
    if (format !== "png" || !hasPngSignature(fileBytes)) return undefined;
    const bytes = Uint8Array.from(fileBytes);
    let binary = "";
    const chunkSize = 0x8000;
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      binary += String.fromCharCode(
        ...bytes.subarray(offset, offset + chunkSize),
      );
    }
    return `data:image/png;base64,${btoa(binary)}`;
  }, [fileBytes, format]);
}

function hasPngSignature(fileBytes: readonly number[]): boolean {
  const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  return signature.every((byte, index) => fileBytes[index] === byte);
}

function importSnapshotFormat(fileName: string): SnapshotFormat {
  return fileName.toLocaleLowerCase().endsWith(".png") ? "png" : "json";
}

function importSnapshotMemoryLevel(memoryLevel: string): SnapshotMemoryLevel {
  return memoryLevel === "core" || memoryLevel === "everything"
    ? memoryLevel
    : "none";
}

function importSetupValues(
  preview: AgentSnapshotImportPreview,
): CreatePersonaInput {
  const respondTo = safeImportedRespondTo(preview.respondTo);
  const behavior =
    respondTo !== undefined || preview.parallelism !== null
      ? {
          respondTo,
          parallelism: preview.parallelism ?? undefined,
        }
      : undefined;
  return {
    displayName: preview.displayName,
    avatarUrl: preview.avatarUrl ?? undefined,
    systemPrompt: preview.systemPrompt ?? "",
    runtime: preview.runtime ?? undefined,
    model: preview.model ?? undefined,
    provider: preview.provider ?? undefined,
    namePool: preview.namePool,
    behavior,
  };
}

function safeImportedRespondTo(
  respondTo: string | null,
): "owner-only" | "anyone" | undefined {
  if (respondTo === "anyone") return "anyone";
  if (respondTo === "allowlist") return "owner-only";
  return respondTo === "owner-only" ? "owner-only" : undefined;
}

// ── Result body ───────────────────────────────────────────────────────────────

export function ResultBody({
  result,
  confirmError,
}: {
  result: AgentSnapshotImportResult;
  confirmError: string | null;
}) {
  const hasPartialMemory =
    result.memoryTotal > 0 && result.memoryWritten < result.memoryTotal;

  return (
    <div className="space-y-3 py-1">
      <p className="text-sm">
        <span className="font-medium">{result.displayName}</span> was created
        successfully.
      </p>

      {result.memoryTotal > 0 ? (
        hasPartialMemory ? (
          <div
            className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
            data-testid="agent-snapshot-import-partial-memory"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="flex flex-col gap-1">
              <p>
                Memory partially restored: {result.memoryWritten} of{" "}
                {result.memoryTotal} entr
                {result.memoryTotal === 1 ? "y" : "ies"} written. The agent
                exists but some memory entries failed to publish.
              </p>
              {result.memoryErrors.length > 0 ? (
                <ul
                  className="mt-1 max-h-32 space-y-0.5 overflow-y-auto text-xs"
                  data-testid="agent-snapshot-import-memory-errors"
                >
                  {result.memoryErrors.map((err) => (
                    <li key={err} className="break-all font-mono">
                      {err}
                    </li>
                  ))}
                </ul>
              ) : null}
            </div>
          </div>
        ) : (
          <p
            className="text-xs text-muted-foreground"
            data-testid="agent-snapshot-import-memory-success"
          >
            {result.memoryTotal} memory entr
            {result.memoryTotal === 1 ? "y" : "ies"} restored.
          </p>
        )
      ) : null}

      {result.profileSyncError ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Profile sync: {result.profileSyncError}
        </p>
      ) : null}

      {confirmError ? (
        <p className="text-xs text-destructive">{confirmError}</p>
      ) : null}
    </div>
  );
}
