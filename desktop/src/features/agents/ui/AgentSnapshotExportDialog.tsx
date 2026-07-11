import * as React from "react";
import { AlertCircle, Download } from "lucide-react";

import type { AgentPersona } from "@/shared/api/types";
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
import { Separator } from "@/shared/ui/separator";

type AgentSnapshotExportDialogProps = {
  isPending: boolean;
  open: boolean;
  persona: AgentPersona;
  /** Pubkey of the linked agent instance to use as the memory source.
   *  When null, memory levels are disabled — the definition has no agent
   *  instance with a keypair to read memory from. */
  linkedAgentPubkey: string | null;
  onExport: (memoryLevel: SnapshotMemoryLevel, format: SnapshotFormat) => void;
  onOpenChange: (open: boolean) => void;
};

const MEMORY_LEVELS: {
  value: SnapshotMemoryLevel;
  label: string;
  description: string;
}[] = [
  {
    value: "none",
    label: "Config only",
    description: "Exports definition and profile — no memory.",
  },
  {
    value: "core",
    label: "Config + core memory",
    description: "Includes the agent's core memory as plaintext.",
  },
  {
    value: "everything",
    label: "Config + all memory",
    description: "Includes core and all mem/* entries as plaintext.",
  },
];

export function AgentSnapshotExportDialog({
  isPending,
  open,
  persona,
  linkedAgentPubkey,
  onExport,
  onOpenChange,
}: AgentSnapshotExportDialogProps) {
  const [memoryLevel, setMemoryLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [format, setFormat] = React.useState<SnapshotFormat>("json");

  const hasLinkedAgent = linkedAgentPubkey !== null;
  const showMemoryWarning = memoryLevel !== "none";
  // PNG format is disabled when memory is selected (backend guards this too,
  // but we disable the option proactively to avoid a confusing error).
  const pngDisabled = memoryLevel !== "none";

  React.useEffect(() => {
    // If the user switches to a memory level, force JSON format since PNG
    // cannot carry memory.
    if (pngDisabled && format === "png") {
      setFormat("json");
    }
  }, [pngDisabled, format]);

  // Reset state when the dialog opens for a fresh export.
  React.useEffect(() => {
    if (open) {
      setMemoryLevel("none");
      setFormat("json");
    }
  }, [open]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-md"
        data-testid="agent-snapshot-export-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>Export agent snapshot</DialogTitle>
            <div className="flex items-center gap-2">
              <Button
                data-testid="agent-snapshot-export-confirm"
                disabled={isPending}
                onClick={() => onExport(memoryLevel, format)}
                size="sm"
                type="button"
                variant="default"
              >
                <Download className="h-4 w-4" />
                Export
              </Button>
              <DialogClose asChild>
                <Button
                  disabled={isPending}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Cancel
                </Button>
              </DialogClose>
            </div>
          </div>
        </DialogHeader>

        <Separator />

        <div className="space-y-4 py-1">
          {/* Agent identity */}
          <p className="text-sm text-muted-foreground">
            Exporting{" "}
            <span className="font-medium text-foreground">
              {persona.displayName}
            </span>{" "}
            as a portable snapshot file. The recipient imports it as a{" "}
            <em>new</em> agent with fresh keys — identity never travels.
          </p>

          {/* Memory level picker */}
          <div className="space-y-2">
            <p className="text-sm font-medium">Memory to include</p>
            <div className="space-y-1">
              {MEMORY_LEVELS.map(({ value, label, description }) => {
                const memoryDisabled = !hasLinkedAgent && value !== "none";
                return (
                  <label
                    className={`flex items-start gap-3 rounded-md px-3 py-2 ${memoryDisabled ? "cursor-not-allowed opacity-40" : "cursor-pointer hover:bg-muted"}`}
                    key={value}
                  >
                    <input
                      checked={memoryLevel === value}
                      className="mt-0.5 shrink-0"
                      disabled={memoryDisabled}
                      name="memory-level"
                      onChange={() => !memoryDisabled && setMemoryLevel(value)}
                      type="radio"
                      value={value}
                    />
                    <div>
                      <p className="text-sm font-medium leading-none">
                        {label}
                      </p>
                      <p className="mt-0.5 text-xs text-muted-foreground">
                        {description}
                      </p>
                    </div>
                  </label>
                );
              })}
            </div>
            {!hasLinkedAgent ? (
              <p className="px-3 text-xs text-muted-foreground">
                Memory export requires a running agent instance. Start this
                definition to enable memory levels.
              </p>
            ) : null}
          </div>

          {/* Plaintext memory warning */}
          {showMemoryWarning ? (
            <div
              className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
              data-testid="agent-snapshot-memory-warning"
            >
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <p>
                Memory is stored as <strong>plaintext</strong> in the snapshot
                file. Only share it with people you trust.
              </p>
            </div>
          ) : null}

          {/* Format picker */}
          <div className="space-y-2">
            <p className="text-sm font-medium">File format</p>
            <div className="flex gap-3">
              <label className="flex cursor-pointer items-center gap-2">
                <input
                  checked={format === "json"}
                  name="snapshot-format"
                  onChange={() => setFormat("json")}
                  type="radio"
                  value="json"
                />
                <span className="text-sm">.agent.json</span>
              </label>
              <label
                className={`flex items-center gap-2 ${pngDisabled ? "cursor-not-allowed opacity-40" : "cursor-pointer"}`}
              >
                <input
                  checked={format === "png"}
                  disabled={pngDisabled}
                  name="snapshot-format"
                  onChange={() => !pngDisabled && setFormat("png")}
                  type="radio"
                  value="png"
                />
                <span className="text-sm">
                  .agent.png
                  {pngDisabled ? " (unavailable with memory)" : ""}
                </span>
              </label>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
