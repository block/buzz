import { History, Pencil, X } from "lucide-react";

import type { Workflow } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { WorkflowDetailPanel } from "./WorkflowDetailPanel";
import { getWorkflowDescription } from "./workflowDefinition";

type WorkflowDetailDialogProps = {
  onEditWorkflow: (workflowId: string) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  workflow: Workflow;
};

export function WorkflowDetailDialog({
  onEditWorkflow,
  onOpenChange,
  open,
  workflow,
}: WorkflowDetailDialogProps) {
  const description = getWorkflowDescription(workflow.definition);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="flex h-[88vh] max-h-[88vh] w-[calc(100vw-2rem)] max-w-6xl flex-col gap-0 overflow-hidden p-0"
        showCloseButton={false}
      >
        <DialogHeader className="flex flex-shrink-0 flex-row items-center justify-between gap-6 space-y-0 px-6 pb-2 pt-3 text-left">
          <div className="min-w-0 space-y-0">
            <DialogTitle className="truncate text-lg leading-tight">
              {workflow.name}
            </DialogTitle>
            <DialogDescription className="truncate font-mono text-sm">
              {description ?? "Workflow details and run history"}
            </DialogDescription>
          </div>
          <div className="flex items-center gap-2">
            <Button
              aria-current="page"
              className="h-8 gap-1.5"
              size="sm"
              type="button"
              variant="secondary"
            >
              <History className="h-4 w-4" />
              Run history
            </Button>
            <Button
              className="h-8 gap-1.5"
              onClick={() => onEditWorkflow(workflow.id)}
              size="sm"
              type="button"
              variant="outline"
            >
              <Pencil className="h-4 w-4" />
              Edit
            </Button>
            <DialogClose asChild>
              <Button
                aria-label="Close"
                className="h-8 w-8 text-muted-foreground"
                size="icon"
                type="button"
                variant="ghost"
              >
                <X className="h-4 w-4" />
              </Button>
            </DialogClose>
          </div>
        </DialogHeader>
        <div className="min-h-0 flex-1">
          <WorkflowDetailPanel
            showDefinition={false}
            showHeader={false}
            workflowId={workflow.id}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
