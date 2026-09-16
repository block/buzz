import { Plus, Workflow as WorkflowIcon } from "lucide-react";

import type { Workflow } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { FieldGroup } from "./ChannelManagementSheetRows";

export function ChannelWorkflowsSection({
  error,
  loading,
  onCreate,
  onOpen,
  onRetry,
  workflows,
}: {
  error: unknown;
  loading: boolean;
  onCreate: () => void;
  onOpen: (workflow: Workflow) => void;
  onRetry: () => void;
  workflows: Workflow[];
}) {
  return (
    <div className="space-y-4 pt-3" data-testid="channel-workflows-section">
      {loading ? (
        <p className="py-8 text-center text-sm text-muted-foreground">
          Loading workflows...
        </p>
      ) : error instanceof Error ? (
        <div className="space-y-3 rounded-xl border border-destructive/30 bg-destructive/10 p-4">
          <p className="text-sm text-destructive">{error.message}</p>
          <Button onClick={onRetry} size="sm" variant="outline">
            Retry
          </Button>
        </div>
      ) : workflows.length > 0 ? (
        <FieldGroup testId="channel-workflows-list">
          {workflows.map((workflow) => (
            <button
              className="flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              data-testid={`channel-workflow-${workflow.id}`}
              key={workflow.id}
              onClick={() => onOpen(workflow)}
              type="button"
            >
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground">
                <WorkflowIcon className="h-4 w-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-foreground">
                  {workflow.name}
                </span>
                <span className="block text-xs capitalize text-muted-foreground">
                  {workflow.status}
                </span>
              </span>
            </button>
          ))}
        </FieldGroup>
      ) : (
        <p className="py-8 text-center text-sm text-muted-foreground">
          No workflows in this channel yet.
        </p>
      )}

      <Button
        className="w-full"
        data-testid="channel-workflows-new"
        onClick={onCreate}
        type="button"
        variant="outline"
      >
        <Plus />
        New workflow
      </Button>
    </div>
  );
}
