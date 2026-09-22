import type { Workflow } from "@/shared/api/types";
import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!isPending) onOpenChange(nextOpen);
      }}
      open={open}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("workflows.delete.title")}</AlertDialogTitle>

          <AlertDialogDescription>
            {workflow
              ? t("workflows.delete.description", { name: workflow.name })
              : t("workflows.delete.description-unnamed")}
          </AlertDialogDescription>
          {error ? (
            <p
              aria-live="polite"
              className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              role="alert"
            >
              {t("workflows.delete.error", { error })}
            </p>
          ) : null}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={isPending} type="button" variant="outline">
              {t("workflows.delete.cancel")}
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
            {isPending
              ? t("workflows.delete.deleting")
              : t("workflows.delete.confirm")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
