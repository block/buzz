import * as React from "react";
import { AlertCircle, Download, Send } from "lucide-react";

import type { AgentTeam } from "@/shared/api/types";
import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriTeams";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Separator } from "@/shared/ui/separator";
import { TeamSnapshotSendDialog } from "./TeamSnapshotSendDialog";

type TeamSnapshotExportDialogProps = {
  isSavePending: boolean;
  open: boolean;
  team: AgentTeam;
  onSaveFile: (
    memoryLevel: SnapshotMemoryLevel,
    format: SnapshotFormat,
  ) => void;
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
    description: "Exports team definition and member profiles — no memory.",
  },
  {
    value: "core",
    label: "Config + core memory",
    description: "Includes each member's core memory as plaintext.",
  },
  {
    value: "everything",
    label: "Config + all memory",
    description: "Includes core and all mem/* entries for every member.",
  },
];

export function TeamSnapshotExportDialog({
  isSavePending,
  open,
  team,
  onSaveFile,
  onOpenChange,
}: TeamSnapshotExportDialogProps) {
  const [memoryLevel, setMemoryLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [format, setFormat] = React.useState<SnapshotFormat>("json");
  const [sendOpen, setSendOpen] = React.useState(false);

  const showMemoryWarning = memoryLevel !== "none";

  // Reset state when the dialog opens for a fresh export.
  React.useEffect(() => {
    if (open) {
      setMemoryLevel("none");
      setFormat("json");
      setSendOpen(false);
    }
  }, [open]);

  const isPending = isSavePending;

  return (
    <>
      <Dialog onOpenChange={onOpenChange} open={open}>
        <DialogContent
          aria-describedby={undefined}
          className="max-w-md"
          data-testid="team-snapshot-export-dialog"
          showCloseButton={false}
        >
          <DialogHeader className="space-y-0">
            <div className="flex items-center justify-between gap-4">
              <DialogTitle>Export team snapshot</DialogTitle>
              <div className="flex items-center gap-2">
                {/* Primary: Send in Buzz */}
                <Button
                  data-testid="team-snapshot-send-in-buzz"
                  disabled={isPending}
                  onClick={() => setSendOpen(true)}
                  size="sm"
                  type="button"
                  variant="default"
                >
                  <Send className="h-4 w-4" />
                  Send in Buzz
                </Button>
                {/* Secondary: Save file */}
                <Button
                  data-testid="team-snapshot-export-confirm"
                  disabled={isPending}
                  onClick={() => onSaveFile(memoryLevel, format)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <Download className="h-4 w-4" />
                  Save file
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
            {/* Team identity */}
            <p className="text-sm text-muted-foreground">
              Exporting{" "}
              <span className="font-medium text-foreground">{team.name}</span>{" "}
              as a portable snapshot. The recipient imports it as a <em>new</em>{" "}
              team with fresh keys — identity never travels.
            </p>

            {/* Memory level picker */}
            <div className="space-y-2">
              <p className="text-sm font-medium">Memory to include</p>
              <div className="space-y-1">
                {MEMORY_LEVELS.map(({ value, label, description }) => (
                  <label
                    className="flex cursor-pointer items-start gap-3 rounded-md px-3 py-2 hover:bg-muted"
                    key={value}
                  >
                    <input
                      checked={memoryLevel === value}
                      className="mt-0.5 shrink-0"
                      name="memory-level"
                      onChange={() => setMemoryLevel(value)}
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
                ))}
              </div>
            </div>

            {/* Plaintext memory warning */}
            {showMemoryWarning ? (
              <div
                className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
                data-testid="team-snapshot-memory-warning"
              >
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                <p>
                  Memory is stored as <strong>plaintext</strong> in the
                  snapshot. Only share it with people you trust.
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
                  <span className="text-sm">.team.json</span>
                </label>
                <label className="flex cursor-pointer items-center gap-2">
                  <input
                    checked={format === "png"}
                    name="snapshot-format"
                    onChange={() => setFormat("png")}
                    type="radio"
                    value="png"
                  />
                  <span className="text-sm">.team.png</span>
                </label>
              </div>
              <p className="text-xs text-muted-foreground">
                Applies to saved files; snapshots shared in Buzz always use
                .team.png. PNG exports include memory when selected.
              </p>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* Send-in-Buzz destination picker — opened as a secondary dialog */}
      {sendOpen ? (
        <TeamSnapshotSendDialog
          memoryLevel={memoryLevel}
          open={sendOpen}
          team={team}
          onOpenChange={(open) => {
            setSendOpen(open);
          }}
          onSent={() => {
            setSendOpen(false);
            onOpenChange(false);
          }}
        />
      ) : null}
    </>
  );
}
