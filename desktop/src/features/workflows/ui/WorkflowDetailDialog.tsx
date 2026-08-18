import { History, Pencil, Play, X } from "lucide-react";
import * as React from "react";
import { stringify as yamlStringify } from "yaml";

import { useTriggerWorkflowMutation } from "@/features/workflows/hooks";
import type { Channel, Workflow } from "@/shared/api/types";
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
import { WorkflowFormBuilder } from "./WorkflowFormBuilder";
import { yamlToFormState } from "./workflowFormTypes";
import { getWorkflowDescription } from "./workflowDefinition";

type WorkflowDetailDialogProps = {
  channels: Channel[];
  onEditWorkflow: (workflowId: string) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  workflow: Workflow;
};

export function WorkflowDetailDialog({
  channels,
  onEditWorkflow,
  onOpenChange,
  open,
  workflow,
}: WorkflowDetailDialogProps) {
  const description = getWorkflowDescription(workflow.definition);
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const yaml = React.useMemo(
    () => yamlStringify(workflow.definition),
    [workflow.definition],
  );
  const formParse = React.useMemo(() => yamlToFormState(yaml), [yaml]);
  const triggerMutation = useTriggerWorkflowMutation(workflow.id);
  const triggerError =
    triggerMutation.error instanceof Error &&
    triggerMutation.error.message.trim().length > 0
      ? triggerMutation.error.message
      : "The relay did not create a workflow run.";

  async function handleTrigger() {
    try {
      await triggerMutation.mutateAsync();
    } catch {
      // React Query retains the error for the inline alert.
    }
  }

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
              className="h-8 gap-1.5"
              disabled={triggerMutation.isPending}
              onClick={() => void handleTrigger()}
              size="sm"
              type="button"
              variant="outline"
            >
              <Play className="h-4 w-4" />
              {triggerMutation.isPending ? "Triggering..." : "Trigger"}
            </Button>
            <Button
              aria-pressed={historyOpen}
              className="h-8 gap-1.5"
              onClick={() => setHistoryOpen((value) => !value)}
              size="sm"
              type="button"
              variant={historyOpen ? "secondary" : "outline"}
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
        {triggerMutation.isError ? (
          <div
            className="border-y px-6 py-2 text-xs text-destructive"
            role="alert"
          >
            <p className="font-medium">Failed to trigger workflow</p>
            <p className="mt-1 break-words text-muted-foreground">
              {triggerError}
            </p>
          </div>
        ) : null}
        <div className="min-h-0 flex-1">
          <WorkflowFormBuilder
            channels={channels}
            disabled
            historyPanel={
              historyOpen ? (
                <WorkflowDetailPanel
                  showDefinition={false}
                  showHeader={false}
                  workflowId={workflow.id}
                />
              ) : null
            }
            mode={formParse.ok ? "form" : "yaml"}
            onChange={() => {}}
            onHistoryClose={() => setHistoryOpen(false)}
            onSelectedNodeChange={() => {}}
            parseError={formParse.ok ? null : formParse.error}
            selectedNode={null}
            workflowChannelId={workflow.channelId}
            yaml={yaml}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
