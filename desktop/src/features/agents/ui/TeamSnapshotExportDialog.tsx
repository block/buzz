import * as React from "react";
import { AlertCircle, Brain, Download, FileType2 } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriTeams";
import type { AgentTeam } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

import { SnapshotOptionMenu } from "./SnapshotOptionMenu";

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
}[] = [
  { value: "none", label: "Team only" },
  { value: "core", label: "Team + core memory" },
  { value: "everything", label: "Team + all memories" },
];

const FORMAT_OPTIONS: { value: SnapshotFormat; label: string }[] = [
  { value: "json", label: "JSON" },
  { value: "png", label: "PNG" },
];

const MODAL_RESIZE_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function TeamSnapshotExportDialog({
  isSavePending,
  open,
  team,
  onSaveFile,
  onOpenChange,
}: TeamSnapshotExportDialogProps) {
  const [memoryLevel, setMemoryLevel] =
    React.useState<SnapshotMemoryLevel>("none");
  const [format, setFormat] = React.useState<SnapshotFormat>("png");
  const shouldReduceMotion = useReducedMotion();
  const showMemoryWarning = memoryLevel !== "none";
  const modalResizeTransition = shouldReduceMotion
    ? { duration: 0 }
    : MODAL_RESIZE_TRANSITION;

  React.useEffect(() => {
    if (open) {
      setMemoryLevel("none");
      setFormat("png");
    }
  }, [open]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-md"
        data-testid="team-snapshot-export-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <DialogTitle className="truncate">Export {team.name}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4 pt-2">
          <div className="space-y-1">
            <div className="flex min-h-8 items-center justify-between gap-4">
              <span className="flex min-w-0 items-center gap-2 text-sm font-medium">
                <Brain className="h-4 w-4 shrink-0 text-muted-foreground" />
                Memories
              </span>
              <SnapshotOptionMenu
                ariaLabel="Memories"
                className="font-medium text-foreground"
                disabled={isSavePending}
                onValueChange={(value) =>
                  setMemoryLevel(value as SnapshotMemoryLevel)
                }
                options={MEMORY_LEVELS}
                testId="team-snapshot-memory-trigger"
                value={memoryLevel}
              />
            </div>

            <div className="flex min-h-8 items-center justify-between gap-4">
              <span className="flex min-w-0 items-center gap-2 text-sm font-medium">
                <FileType2 className="h-4 w-4 shrink-0 text-muted-foreground" />
                File format
              </span>
              <SnapshotOptionMenu
                ariaLabel="File format"
                className="font-medium text-foreground"
                disabled={isSavePending}
                onValueChange={(value) => setFormat(value as SnapshotFormat)}
                options={FORMAT_OPTIONS}
                testId="team-snapshot-format-trigger"
                value={format}
              />
            </div>
          </div>

          <AnimatePresence initial={false}>
            {showMemoryWarning ? (
              <motion.div
                animate={{ height: "auto", opacity: 1 }}
                className="overflow-hidden"
                data-testid="team-snapshot-memory-warning-motion"
                exit={{ height: 0, opacity: 0 }}
                initial={{ height: 0, opacity: 0 }}
                key="team-snapshot-memory-warning"
                transition={modalResizeTransition}
              >
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
              </motion.div>
            ) : null}
          </AnimatePresence>

          <div
            className="flex items-center justify-end gap-2 pt-2"
            data-testid="team-snapshot-export-footer"
          >
            <DialogClose asChild>
              <Button
                disabled={isSavePending}
                size="sm"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
            </DialogClose>
            <Button
              data-testid="team-snapshot-export-confirm"
              disabled={isSavePending}
              onClick={() => onSaveFile(memoryLevel, format)}
              size="sm"
              type="button"
            >
              <Download className="h-4 w-4" />
              Export
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
