import * as React from "react";
import { AlertCircle, Upload } from "lucide-react";

import type {
  AgentSnapshotImportPreview,
  AgentSnapshotImportResult,
} from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Separator } from "@/shared/ui/separator";

// ── Types ─────────────────────────────────────────────────────────────────────

type ImportPhase = "preview" | "confirming" | "result";

const SYSTEM_PROMPT_PREVIEW_CHARS = 180;

type AgentSnapshotImportDialogProps = {
  open: boolean;
  /** Preview data loaded by the caller before opening. */
  preview: AgentSnapshotImportPreview;
  /** True while the confirm mutation is in-flight. */
  isConfirming: boolean;
  /** Set when the confirm mutation has returned a result. */
  result: AgentSnapshotImportResult | null;
  /** Error from the confirm mutation, if any. */
  confirmError: string | null;
  /** Called with keepAllowlist when user clicks Import. */
  onConfirm: (keepAllowlist: boolean) => void;
  onOpenChange: (open: boolean) => void;
};

// ── Component ─────────────────────────────────────────────────────────────────

export function AgentSnapshotImportDialog({
  open,
  preview,
  isConfirming,
  result,
  confirmError,
  onConfirm,
  onOpenChange,
}: AgentSnapshotImportDialogProps) {
  // Default: clear the source allowlist (safe default per spec).
  const [keepAllowlist, setKeepAllowlist] = React.useState(false);
  const [showFullPrompt, setShowFullPrompt] = React.useState(false);

  // Reset preview choices whenever the dialog opens.
  React.useEffect(() => {
    if (open) {
      setKeepAllowlist(false);
      setShowFullPrompt(false);
    }
  }, [open]);

  const phase: ImportPhase =
    result !== null ? "result" : isConfirming ? "confirming" : "preview";

  const hasMemory = preview.memoryEntryCount > 0;
  const memoryLevelLabel =
    preview.memoryLevel === "core"
      ? "core"
      : preview.memoryLevel === "everything"
        ? "all"
        : "none";

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-md"
        data-testid="agent-snapshot-import-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>
              {phase === "result" ? "Agent imported" : "Import agent snapshot"}
            </DialogTitle>
            <div className="flex items-center gap-2">
              {phase === "preview" ? (
                <>
                  <Button
                    data-testid="agent-snapshot-import-confirm"
                    disabled={isConfirming}
                    onClick={() => onConfirm(keepAllowlist)}
                    size="sm"
                    type="button"
                    variant="default"
                  >
                    <Upload className="h-4 w-4" />
                    Import
                  </Button>
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
                </>
              ) : (
                <DialogClose asChild>
                  <Button size="sm" type="button" variant="ghost">
                    Close
                  </Button>
                </DialogClose>
              )}
            </div>
          </div>
        </DialogHeader>

        <Separator />

        {phase === "preview" ? (
          <PreviewBody
            preview={preview}
            hasMemory={hasMemory}
            memoryLevelLabel={memoryLevelLabel}
            keepAllowlist={keepAllowlist}
            onKeepAllowlistChange={setKeepAllowlist}
            onShowFullPromptChange={setShowFullPrompt}
            showFullPrompt={showFullPrompt}
          />
        ) : phase === "confirming" ? (
          <div className="py-4 text-center text-sm text-muted-foreground">
            Creating agent…
          </div>
        ) : result !== null ? (
          <ResultBody result={result} confirmError={confirmError} />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

// ── Preview body ──────────────────────────────────────────────────────────────

function PreviewBody({
  preview,
  hasMemory,
  memoryLevelLabel,
  keepAllowlist,
  onKeepAllowlistChange,
  onShowFullPromptChange,
  showFullPrompt,
}: {
  preview: AgentSnapshotImportPreview;
  hasMemory: boolean;
  memoryLevelLabel: string;
  keepAllowlist: boolean;
  onKeepAllowlistChange: (v: boolean) => void;
  onShowFullPromptChange: (show: boolean) => void;
  showFullPrompt: boolean;
}) {
  return (
    <div className="space-y-4 py-1">
      {/* Agent identity */}
      <div className="space-y-1">
        <p className="text-sm font-medium">{preview.displayName}</p>
        {preview.systemPrompt ? (
          <SystemPromptReview
            onExpandedChange={onShowFullPromptChange}
            prompt={preview.systemPrompt}
            expanded={showFullPrompt}
          />
        ) : null}
      </div>

      <p className="text-sm text-muted-foreground">
        A new agent will be created with a fresh keypair. The imported agent is
        independent of the source — identity never travels.
      </p>

      {/* Memory section */}
      {hasMemory ? (
        <div
          className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
          data-testid="agent-snapshot-import-memory-warning"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <p>
            This snapshot includes{" "}
            <strong>
              {preview.memoryEntryCount} {memoryLevelLabel} memory entr
              {preview.memoryEntryCount === 1 ? "y" : "ies"}
            </strong>
            . Memory is stored as plaintext in the file and will be restored
            under the new agent's identity.
          </p>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          No memory included — config only.
        </p>
      )}

      {/* Allowlist section */}
      {preview.hasSourceAllowlist ? (
        <div
          className="space-y-2 rounded-md border border-border p-3"
          data-testid="agent-snapshot-import-allowlist-section"
        >
          <p className="text-sm font-medium">
            Respond-to allowlist ({preview.sourceAllowlistCount} entr
            {preview.sourceAllowlistCount === 1 ? "y" : "ies"})
          </p>
          <p className="text-xs text-muted-foreground">
            This snapshot includes a source-environment pubkey allowlist. Those
            identities are not meaningful on your relay.
          </p>
          <div className="flex flex-col gap-1.5">
            <label className="flex cursor-pointer items-center gap-2">
              <input
                checked={!keepAllowlist}
                data-testid="agent-snapshot-import-allowlist-clear"
                name="allowlist-choice"
                onChange={() => onKeepAllowlistChange(false)}
                type="radio"
              />
              <span className="text-sm">
                <strong>Clear</strong> — start with an empty allowlist (safer)
              </span>
            </label>
            <label className="flex cursor-pointer items-center gap-2">
              <input
                checked={keepAllowlist}
                data-testid="agent-snapshot-import-allowlist-keep"
                name="allowlist-choice"
                onChange={() => onKeepAllowlistChange(true)}
                type="radio"
              />
              <span className="text-sm">
                <strong>Keep</strong> — copy source allowlist to the new agent
              </span>
            </label>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function SystemPromptReview({
  prompt,
  expanded,
  onExpandedChange,
}: {
  prompt: string;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
}) {
  const hasMore = prompt.length > SYSTEM_PROMPT_PREVIEW_CHARS;
  const preview = hasMore
    ? `${prompt.slice(0, SYSTEM_PROMPT_PREVIEW_CHARS).trimEnd()}…`
    : prompt;

  return (
    <div className="space-y-1.5">
      <p className="text-xs font-medium text-muted-foreground">
        Instructions · {prompt.length.toLocaleString()} characters
      </p>
      {expanded ? (
        <div
          className="max-h-64 overflow-y-auto rounded-md border border-border bg-muted/30 p-3"
          data-testid="agent-snapshot-import-full-prompt"
        >
          <p className="whitespace-pre-wrap wrap-break-word text-xs text-muted-foreground">
            {prompt}
          </p>
        </div>
      ) : (
        <p
          className="line-clamp-3 whitespace-pre-wrap wrap-break-word text-xs text-muted-foreground"
          data-testid="agent-snapshot-import-prompt-excerpt"
        >
          {preview}
        </p>
      )}
      {hasMore ? (
        <Button
          aria-expanded={expanded}
          className="-ml-2 h-auto px-2 py-1"
          data-testid="agent-snapshot-import-prompt-toggle"
          onClick={() => onExpandedChange(!expanded)}
          size="sm"
          type="button"
          variant="ghost"
        >
          {expanded ? "Hide full instructions" : "Review full instructions"}
        </Button>
      ) : null}
    </div>
  );
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
