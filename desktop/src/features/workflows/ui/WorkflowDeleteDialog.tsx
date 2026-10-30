// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import type { Workflow } from "@/shared/api/types";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type WorkflowDeleteDialogProps = {
  error?: string | null;
  isPending?: boolean;
  open: boolean;
  workflow: Workflow | null;
  onConfirm: (workflow: Workflow) => Promise<void>;
  onOpenChange: (open: boolean) => void;
};

export function WorkflowDeleteDialog({
  error,
  isPending = false,
  open,
  workflow,
  onConfirm,
  onOpenChange,
}: WorkflowDeleteDialogProps) {
  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!isPending) onOpenChange(nextOpen);
      }}
      open={open}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{uiT("Delete workflow?")}</AlertDialogTitle>
          <AlertDialogDescription>
            {workflow
              ? uiT(
                  'Delete "{0}". This removes the current workflow and stops new triggers. Publishing its definition again, even an older version, can recreate it.',
                  { p0: workflow.name },
                )
              : uiT("Delete this workflow.")}
          </AlertDialogDescription>
          {error ? (
            <p
              aria-live="polite"
              className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              role="alert"
            >
              {uiT(
                "Couldn’t delete workflow. {0} Try again or cancel to keep editing.",
                { p0: error },
              )}
            </p>
          ) : null}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={isPending} type="button" variant="outline">
              {uiT("Cancel")}
            </Button>
          </AlertDialogCancel>
          <Button
            disabled={!workflow || isPending}
            onClick={() => {
              if (workflow) {
                void onConfirm(workflow);
              }
            }}
            type="button"
            variant="destructive"
          >
            {isPending ? uiT("Deleting…") : uiT("Delete")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
