import * as React from "react";
import { AlertTriangle, ChevronDown, ChevronRight } from "lucide-react";

import {
  ImportStatusIcon,
  type ImportItemStatus,
} from "@/shared/ui/import-status-icon";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { ParsePersonaFilesResult } from "@/shared/api/tauriPersonas";
import { createPersona } from "@/shared/api/tauriPersonas";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type BatchImportDialogProps = {
  fileName: string;
  open: boolean;
  result: ParsePersonaFilesResult | null;
  onOpenChange: (open: boolean) => void;
  onComplete: (count: number) => void;
};

export function BatchImportDialog({
  fileName,
  open,
  result,
  onOpenChange,
  onComplete,
}: BatchImportDialogProps) {
  const [selected, setSelected] = React.useState<Set<number>>(new Set());
  const [status, setStatus] = React.useState<
    "idle" | "importing" | "done" | "error"
  >("idle");
  const [importedCount, setImportedCount] = React.useState(0);
  const [itemStatuses, setItemStatuses] = React.useState<
    Map<number, ImportItemStatus>
  >(new Map());
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [expandedIndex, setExpandedIndex] = React.useState<number | null>(null);
  const [skippedExpanded, setSkippedExpanded] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      return;
    }

    const count = result?.personas.length ?? 0;
    setSelected(new Set(Array.from({ length: count }, (_, i) => i)));
    setStatus("idle");
    setImportedCount(0);
    setItemStatuses(new Map());
    setErrorMessage(null);
    setExpandedIndex(null);
    setSkippedExpanded(false);
  }, [open, result]);

  async function handleImport() {
    if (!result) {
      return;
    }

    setStatus("importing");
    setErrorMessage(null);

    // Initialize all selected items as pending
    const initialStatuses = new Map<number, ImportItemStatus>();
    for (const index of selected) {
      initialStatuses.set(index, "pending");
    }
    setItemStatuses(new Map(initialStatuses));

    let completed = 0;

    for (const index of selected) {
      const persona = result.personas[index];
      if (!persona) {
        continue;
      }

      setItemStatuses((prev) => {
        const next = new Map(prev);
        next.set(index, "importing");
        return next;
      });

      try {
        await createPersona({
          displayName: persona.displayName,
          avatarUrl: persona.avatarDataUrl ?? undefined,
          systemPrompt: persona.systemPrompt,
          runtime: persona.runtime ?? undefined,
          model: persona.model ?? undefined,
        });
        completed += 1;
        setImportedCount(completed);
        setItemStatuses((prev) => {
          const next = new Map(prev);
          next.set(index, "done");
          return next;
        });
      } catch (error) {
        setItemStatuses((prev) => {
          const next = new Map(prev);
          next.set(index, "error");
          return next;
        });
        setStatus("error");
        setErrorMessage(
          `Imported ${completed} of ${selected.size}. Failed on '${persona.displayName}': ${error instanceof Error ? error.message : String(error)}. Already-imported personas are saved.`,
        );
        return;
      }
    }

    setStatus("done");
    onComplete(completed);
  }

  const personas = result?.personas ?? [];
  const skipped = result?.skipped ?? [];
  const selectedCount = selected.size;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-2xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Import Personas</DialogTitle>
            <DialogDescription>
              Found {personas.length} persona
              {personas.length !== 1 ? "s" : ""} in {fileName || "archive"}.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
            <div className="space-y-1">
              {personas.map((persona, index) => {
                const isExpanded = expandedIndex === index;
                const isSelected = selected.has(index);
                const firstLine = persona.systemPrompt
                  .trim()
                  .split("\n")
                  .find((line) => line.trim().length > 0);

                return (
                  <div
                    className="rounded-lg border border-border/60 bg-card/80"
                    key={persona.sourceFile}
                  >
                    <button
                      className="flex w-full items-center gap-3 px-3 py-2.5 text-left"
                      onClick={() =>
                        setExpandedIndex(isExpanded ? null : index)
                      }
                      type="button"
                    >
                      <Checkbox
                        checked={isSelected}
                        disabled={status === "importing"}
                        onCheckedChange={(checked: boolean) => {
                          setSelected((prev) => {
                            const next = new Set(prev);
                            if (checked) {
                              next.add(index);
                            } else {
                              next.delete(index);
                            }
                            return next;
                          });
                        }}
                        onClick={(e: React.MouseEvent) => e.stopPropagation()}
                      />
                      <ProfileAvatar
                        avatarUrl={persona.avatarDataUrl}
                        className="h-8 w-8 rounded-lg text-xs"
                        label={persona.displayName}
                      />
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-semibold tracking-tight">
                          {persona.displayName}
                        </p>
                        {firstLine && !isExpanded ? (
                          <p className="truncate text-xs text-muted-foreground">
                            {firstLine}
                          </p>
                        ) : null}
                      </div>
                      <ImportStatusIcon status={itemStatuses.get(index)} />
                    </button>
                    {isExpanded ? (
                      <div className="border-t border-border/40 px-3 py-2.5">
                        <pre className="whitespace-pre-wrap text-xs text-muted-foreground">
                          {persona.systemPrompt}
                        </pre>
                      </div>
                    ) : null}
                  </div>
                );
              })}
            </div>

            {skipped.length > 0 ? (
              <div className="mt-4">
                <button
                  className="flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
                  onClick={() => setSkippedExpanded((prev) => !prev)}
                  type="button"
                >
                  <AlertTriangle className="h-4 w-4" />
                  {skipped.length} file{skipped.length !== 1 ? "s" : ""} skipped
                  {skippedExpanded ? (
                    <ChevronDown className="h-4 w-4" />
                  ) : (
                    <ChevronRight className="h-4 w-4" />
                  )}
                </button>
                {skippedExpanded ? (
                  <div className="mt-2 space-y-1">
                    {skipped.map((file) => (
                      <p
                        className="text-xs text-muted-foreground"
                        key={file.sourceFile}
                      >
                        <span className="font-medium">{file.sourceFile}</span>
                        {" — "}
                        {file.reason}
                      </p>
                    ))}
                  </div>
                ) : null}
              </div>
            ) : null}

            {errorMessage ? (
              <p className="mt-4 rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {errorMessage}
              </p>
            ) : null}
          </div>

          <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
            <Button
              onClick={() => onOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              disabled={selectedCount === 0 || status === "importing"}
              onClick={() => void handleImport()}
              size="sm"
              type="button"
            >
              {status === "importing"
                ? `Importing ${importedCount}/${selectedCount}...`
                : `Import ${selectedCount}`}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
