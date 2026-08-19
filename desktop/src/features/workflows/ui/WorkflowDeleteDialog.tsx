import type { Workflow } from "@/shared/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type WorkflowDeleteDialogProps = {
  open: boolean;
  workflow: Workflow | null;
  onConfirm: (workflow: Workflow) => void;
  onOpenChange: (open: boolean) => void;
};

export function WorkflowDeleteDialog({
  open,
  workflow,
  onConfirm,
  onOpenChange,
}: WorkflowDeleteDialogProps) {
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete workflow?</AlertDialogTitle>
          <AlertDialogDescription>
            {workflow
              ? `Delete "${workflow.name}". This will stop all future triggers and remove the workflow permanently.`
              : "Delete this workflow."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              onClick={() => {
                if (workflow) {
                  onConfirm(workflow);
                }
              }}
              type="button"
              variant="destructive"
            >
              Delete
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
