import * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { ParsedPersonaPreview } from "@/shared/api/tauriPersonas";
import type { AgentPersona } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  buildPersonaImportPlan,
  hasAnyPersonaImportChanges,
} from "./personaImportPlan";
import {
  getFieldPreview,
  getFieldSecondaryText,
} from "./personaImportUpdateState";

type PersonaImportUpdateDialogProps = {
  open: boolean;
  persona: AgentPersona | null;
  preview: ParsedPersonaPreview | null;
  fileName: string;
  isPending: boolean;
  onClear: () => void;
  onOpenChange: (open: boolean) => void;
  onApply: (input: { selectedFields: string[] }) => Promise<void>;
};

export function PersonaImportUpdateDialog({
  open,
  persona,
  preview,
  fileName,
  isPending,
  onClear,
  onOpenChange,
  onApply,
}: PersonaImportUpdateDialogProps) {
  const [selectedFields, setSelectedFields] = React.useState<Set<string>>(
    new Set(),
  );
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  const plan = React.useMemo(() => {
    if (!persona || !preview) {
      return null;
    }
    return buildPersonaImportPlan({ persona, preview });
  }, [persona, preview]);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setErrorMessage(null);
    setSelectedFields(new Set());
  }, [open]);

  React.useEffect(() => {
    if (!open || !plan) {
      return;
    }
    setSelectedFields(new Set(plan.fields.map((field) => field.field)));
  }, [open, plan]);

  function toggleField(field: string, checked: boolean) {
    setSelectedFields((current) => {
      const next = new Set(current);
      if (checked) {
        next.add(field);
      } else {
        next.delete(field);
      }
      return next;
    });
  }

  async function runApply() {
    setErrorMessage(null);
    try {
      await onApply({
        selectedFields: Array.from(selectedFields),
      });
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to apply imported persona update.",
      );
    }
  }

  const hasChanges = hasAnyPersonaImportChanges(plan);
  const selectedCount = selectedFields.size;

  function renderLineChangeSummary(
    addedLines: number,
    removedLines: number,
    emphasize = true,
  ) {
    const addedClass = emphasize
      ? addedLines > 0
        ? "text-status-added"
        : "text-muted-foreground"
      : "text-muted-foreground";
    const separatorClass = "text-muted-foreground";
    const removedClass = emphasize
      ? removedLines > 0
        ? "text-status-deleted"
        : "text-muted-foreground"
      : "text-muted-foreground";
    const opacityClass = emphasize ? "opacity-100" : "opacity-50";

    return (
      <p
        className={`shrink-0 text-xs font-medium tabular-nums transition-opacity ${opacityClass}`}
      >
        <span className={addedClass}>+{addedLines}</span>
        <span className={separatorClass}> / </span>
        <span className={removedClass}>-{removedLines}</span>
      </p>
    );
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="flex max-h-[85vh] max-w-3xl flex-col overflow-hidden p-0">
        <DialogHeader className="shrink-0 space-y-1 border-b border-border/60 px-6 py-5 pr-14">
          <DialogTitle>Import agent</DialogTitle>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <div className="space-y-4">
            <div className="flex items-center justify-between gap-3 rounded-md border border-border/60 bg-card/80 px-3 py-2">
              <p className="min-w-0 truncate text-sm font-medium">
                {fileName || "Imported file"}
              </p>
              <button
                aria-label="Clear import"
                className="shrink-0 text-sm text-primary underline-offset-4 hover:underline"
                disabled={isPending}
                onClick={onClear}
                type="button"
              >
                Clear
              </button>
            </div>

            {preview && plan ? (
              <div className="space-y-4">
                {hasChanges ? (
                  <div className="space-y-1">
                    <p className="text-sm font-medium">
                      Fields that will be updated{" "}
                      <span className="font-bold">
                        ({selectedCount}/{plan.fields.length})
                      </span>
                    </p>
                    <div className="space-y-1">
                      {plan.fields.map((fieldChange) => {
                        const shouldUpdate = selectedFields.has(
                          fieldChange.field,
                        );
                        const previewText = getFieldSecondaryText(
                          shouldUpdate,
                          getFieldPreview(
                            fieldChange.importedValue,
                            `No ${fieldChange.label.toLowerCase()} in import.`,
                          ),
                          getFieldPreview(
                            fieldChange.existingValue,
                            `No current ${fieldChange.label.toLowerCase()}.`,
                          ),
                        );

                        return (
                          <div
                            className="flex items-center gap-3 rounded-lg border border-border/60 bg-card/80 px-3 py-2.5"
                            key={fieldChange.field}
                          >
                            <Checkbox
                              checked={shouldUpdate}
                              disabled={isPending}
                              onCheckedChange={(checked) =>
                                toggleField(fieldChange.field, Boolean(checked))
                              }
                            />
                            {fieldChange.field === "avatarUrl" ? (
                              <ProfileAvatar
                                avatarUrl={
                                  shouldUpdate
                                    ? fieldChange.importedValue || null
                                    : fieldChange.existingValue || null
                                }
                                className="h-8 w-8 rounded-lg text-xs"
                                label={persona?.displayName ?? ""}
                              />
                            ) : null}
                            <div className="min-w-0 flex-1">
                              <p className="truncate text-sm font-semibold tracking-tight">
                                {fieldChange.label}
                              </p>
                              <p
                                className={`truncate text-xs ${
                                  shouldUpdate
                                    ? "text-foreground"
                                    : "text-muted-foreground"
                                }`}
                              >
                                {previewText}
                              </p>
                            </div>
                            {renderLineChangeSummary(
                              fieldChange.addedLines,
                              fieldChange.removedLines,
                              shouldUpdate,
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                ) : (
                  <div className="rounded-lg border border-dashed border-border/60 bg-card/60 px-4 py-10 text-center">
                    <p className="text-sm font-semibold tracking-tight text-muted-foreground">
                      no changes
                    </p>
                  </div>
                )}
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">
                No import preview is available.
              </p>
            )}

            {errorMessage ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {errorMessage}
              </p>
            ) : null}
          </div>
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
            disabled={!preview || isPending || selectedCount === 0}
            onClick={() => void runApply()}
            size="sm"
            type="button"
          >
            Apply update
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
