import { ChevronDown, ChevronRight, Pencil, Play, X } from "lucide-react";
import * as React from "react";

import {
  useRunApprovalsQuery,
  useTriggerWorkflowMutation,
  useWorkflowQuery,
  useWorkflowRunsQuery,
} from "@/features/workflows/hooks";
import { WorkflowRunTrace } from "@/features/workflows/ui/WorkflowRunTrace";
import type { Workflow } from "@/shared/api/types";
import { Badge, type BadgeProps } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import {
  getWorkflowDescription,
  getWorkflowDisplayStatus,
  getWorkflowTriggerSummary,
} from "./workflowDefinition";

type WorkflowDetailPanelProps = {
  workflowId: string;
  onClose: () => void;
  onEdit: (workflow: Workflow) => void;
};

export function WorkflowDetailPanel({
  workflowId,
  onClose,
  onEdit,
}: WorkflowDetailPanelProps) {
  const workflowQuery = useWorkflowQuery(workflowId);
  const runsQuery = useWorkflowRunsQuery(workflowId);
  const triggerMutation = useTriggerWorkflowMutation(workflowId);
  const [selectedRunId, setSelectedRunId] = React.useState<string | null>(null);

  const workflow = workflowQuery.data;
  const runs = runsQuery.data ?? [];
  const approvalsQuery = useRunApprovalsQuery(workflowId, selectedRunId);
  const workflowDescription = workflow
    ? getWorkflowDescription(workflow.definition)
    : null;
  const triggerSummary = workflow
    ? getWorkflowTriggerSummary(workflow.definition)
    : null;
  const workflowStatus = workflow ? getWorkflowDisplayStatus(workflow) : null;

  async function handleTrigger() {
    try {
      const response = await triggerMutation.mutateAsync();
      setSelectedRunId(response.runId);
    } catch {
      // React Query stores the error; keep the current selection unchanged.
    }
  }

  return (
    <div
      className="flex h-full flex-col border-l bg-background pt-11"
      data-testid="workflow-detail-panel"
    >
      <div className="flex items-center justify-between border-b px-4 py-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            {workflow ? (
              <h3 className="truncate text-sm font-semibold">
                {workflow.name}
              </h3>
            ) : (
              <Skeleton className="h-4 w-36" />
            )}
            {workflowStatus ? <RunStatusBadge status={workflowStatus} /> : null}
          </div>
          {workflowDescription ? (
            <p className="mt-1 truncate text-xs text-muted-foreground">
              {workflowDescription}
            </p>
          ) : workflowQuery.isLoading ? (
            <Skeleton className="mt-1 h-3 w-full max-w-64" />
          ) : null}
          {triggerSummary ? (
            <p className="mt-1 truncate text-xs text-muted-foreground">
              {triggerSummary}
            </p>
          ) : workflowQuery.isLoading ? (
            <Skeleton className="mt-1 h-3 w-40" />
          ) : null}
        </div>
        <div className="flex items-center gap-1">
          {workflow ? (
            <Button
              onClick={() => onEdit(workflow)}
              size="sm"
              variant="outline"
            >
              <Pencil className="mr-1 h-4 w-4" />
              Edit
            </Button>
          ) : null}
          <Button
            disabled={triggerMutation.isPending || workflowQuery.isLoading}
            onClick={() => void handleTrigger()}
            size="sm"
            variant="outline"
          >
            <Play className="mr-1 h-4 w-4" />
            {triggerMutation.isPending ? "Triggering..." : "Trigger"}
          </Button>
          <Button
            aria-label="Close detail panel"
            onClick={onClose}
            size="icon"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {triggerMutation.isError ? (
        <div className="border-b px-4 py-2 text-xs text-red-400">
          Failed to trigger workflow
        </div>
      ) : null}

      <div
        className="flex-1 overflow-y-auto"
        data-scroll-restoration-id={`workflow-detail:${workflowId}`}
      >
        {workflow ? (
          <div className="space-y-4 p-4">
            <div>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                Definition
              </h4>
              <pre className="max-h-64 overflow-auto rounded-md bg-muted/50 p-3 font-mono text-xs leading-relaxed">
                {JSON.stringify(workflow.definition, null, 2)}
              </pre>
            </div>

            <div>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                Run History
              </h4>
              {runs.length === 0 ? (
                <p className="text-sm text-muted-foreground">No runs yet.</p>
              ) : (
                <div className="space-y-2">
                  {runs.map((run) => {
                    const isSelected = selectedRunId === run.id;
                    const duration = formatRunDuration(
                      run.startedAt,
                      run.completedAt,
                    );

                    return (
                      <div
                        className={`overflow-hidden rounded-xl border bg-card/70 transition-colors ${
                          isSelected
                            ? "border-primary/40 bg-primary/5 shadow-xs"
                            : "border-border/70 hover:bg-muted/20"
                        }`}
                        key={run.id}
                      >
                        <button
                          aria-expanded={isSelected}
                          className="w-full px-4 py-3 text-left"
                          data-testid={
                            isSelected ? "workflow-selected-run" : undefined
                          }
                          onClick={() =>
                            setSelectedRunId(isSelected ? null : run.id)
                          }
                          type="button"
                        >
                          <div className="flex items-start justify-between gap-3">
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-2">
                                {isSelected ? (
                                  <ChevronDown className="h-4 w-4 text-muted-foreground" />
                                ) : (
                                  <ChevronRight className="h-4 w-4 text-muted-foreground" />
                                )}
                                <span className="truncate font-mono text-xs font-medium">
                                  {run.id.slice(0, 8)}
                                </span>
                                <RunStatusBadge status={run.status} />
                              </div>
                              <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 pl-6 text-[11px] text-muted-foreground">
                                <span>
                                  {new Date(
                                    run.createdAt * 1000,
                                  ).toLocaleString()}
                                </span>
                                <span>
                                  {run.executionTrace.length}{" "}
                                  {run.executionTrace.length === 1
                                    ? "step"
                                    : "steps"}
                                </span>
                                {duration ? <span>{duration}</span> : null}
                                {run.currentStep !== null ? (
                                  <span>
                                    Current step {run.currentStep + 1}
                                  </span>
                                ) : null}
                              </div>
                              {run.errorMessage ? (
                                <p className="mt-2 break-words pl-6 text-xs text-destructive">
                                  {run.errorMessage}
                                </p>
                              ) : null}
                            </div>
                          </div>
                        </button>

                        {isSelected ? (
                          <div className="border-t border-border/60 bg-background/60 px-4 py-4">
                            <div className="mb-3 flex items-center gap-2 text-[11px] font-medium uppercase tracking-[0.18em] text-muted-foreground">
                              <span>Execution Trace</span>
                              {approvalsQuery.isFetching ? (
                                <span className="text-[10px] tracking-[0.12em] text-muted-foreground/80">
                                  Refreshing approvals...
                                </span>
                              ) : null}
                            </div>
                            {approvalsQuery.error instanceof Error ? (
                              <p className="mb-3 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                                {approvalsQuery.error.message}
                              </p>
                            ) : null}
                            <WorkflowRunTrace
                              approvals={approvalsQuery.data}
                              run={run}
                            />
                          </div>
                        ) : null}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        ) : workflowQuery.isError ? (
          <div className="flex h-32 flex-col items-center justify-center gap-2">
            <p className="text-sm text-red-400">Failed to load workflow</p>
          </div>
        ) : (
          <div className="space-y-4 p-4">
            <div>
              <Skeleton className="mb-2 h-4 w-28" />
              <Skeleton className="h-40 w-full rounded-xl" />
            </div>
            <div>
              <Skeleton className="mb-2 h-4 w-24" />
              <div className="space-y-2">
                <Skeleton className="h-16 w-full rounded-xl" />
                <Skeleton className="h-16 w-full rounded-xl" />
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function formatRunDuration(
  startedAt: number | null,
  completedAt: number | null,
) {
  if (startedAt === null || completedAt === null) return null;
  const seconds = completedAt - startedAt;
  if (seconds < 1) return `${Math.round(seconds * 1000)}ms`;
  return `${seconds.toFixed(1)}s`;
}

function formatStatusLabel(status: string) {
  return status.replace(/_/g, " ");
}

function RunStatusBadge({ status }: { status: string }) {
  const variants: Record<string, BadgeProps["variant"]> = {
    active: "success",
    disabled: "secondary",
    archived: "warning",
    completed: "success",
    failed: "destructive",
    running: "info",
    pending: "secondary",
    cancelled: "secondary",
    waiting_approval: "warning",
  };

  return (
    <Badge variant={variants[status] ?? "secondary"}>
      {formatStatusLabel(status)}
    </Badge>
  );
}
