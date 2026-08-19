import { Plus, RefreshCw } from "lucide-react";
import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { stringify as yamlStringify } from "yaml";
import { toast } from "sonner";

import {
  allWorkflowsQueryKey,
  workflowListFocusRefetchPolicy,
  workflowQueryKey,
} from "@/features/workflows/hooks";
import { WorkflowCard } from "@/features/workflows/ui/WorkflowCard";
import { WorkflowDeleteDialog } from "@/features/workflows/ui/WorkflowDeleteDialog";
import { WorkflowEditorHost } from "@/features/workflows/ui/WorkflowEditorHost";
import type { WorkflowEditorRoute } from "@/features/workflows/ui/WorkflowsScreen";
import type { WorkflowEditorPane } from "@/features/workflows/ui/workflowEditorPane";
import {
  getWorkflowEnabled,
  withWorkflowEnabled,
} from "@/features/workflows/ui/workflowDefinition";
import type { Channel, Workflow } from "@/shared/api/types";
import {
  deleteWorkflow,
  getChannelsWorkflows,
  triggerWorkflow,
  updateWorkflow,
} from "@/shared/api/tauriWorkflows";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { Skeleton } from "@/shared/ui/skeleton";

type WorkflowsViewProps = {
  channels: Channel[];
  editor: WorkflowEditorRoute | null;
  onCloseEditor: () => void;
  onCreateWorkflow: () => void;
  onDuplicateWorkflow: (workflowId: string) => void;
  onEditWorkflow: (workflowId: string) => void;
  onViewWorkflow: (workflowId: string) => void;
  onEditorPaneChange: (pane: WorkflowEditorPane) => void;
};

type WorkflowWithChannel = {
  workflow: Workflow;
  channelName: string;
};

const WORKFLOW_CARD_GRID_CLASS =
  "grid grid-cols-1 gap-3 [@container(min-width:42rem)]:grid-cols-2 [@container(min-width:63rem)]:grid-cols-3";

function WorkflowsListSkeleton() {
  return (
    <div className={WORKFLOW_CARD_GRID_CLASS}>
      {["first", "second", "third", "fourth"].map((card) => (
        <div
          className="flex min-h-60 flex-col rounded-2xl bg-muted/50 p-5 shadow-xs"
          key={card}
        >
          <div className="flex items-start justify-between">
            <div className="flex items-center gap-2">
              <Skeleton className="h-9 w-9 rounded-xl" />
              <Skeleton className="h-4 w-4" />
              <Skeleton className="h-9 w-9 rounded-xl" />
            </div>
            <Skeleton className="h-6 w-16 rounded-full" />
          </div>
          <Skeleton className="mt-5 h-3 w-28" />
          <Skeleton className="mt-2 h-6 w-full" />
          <Skeleton className="mt-2 h-6 w-4/5" />
          <Skeleton className="mt-auto h-4 w-32" />
        </div>
      ))}
    </div>
  );
}

function CreateWorkflowCard({ onClick }: { onClick: () => void }) {
  return (
    <button
      aria-label="Create Workflow"
      className="group relative flex min-h-60 w-full min-w-0 items-center justify-center overflow-hidden rounded-2xl border border-dashed border-border/80 bg-transparent text-muted-foreground shadow-xs transition-colors hover:border-border hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-testid="new-workflow-card"
      onClick={onClick}
      type="button"
    >
      <Plus className="h-7 w-7 transition-colors" />
    </button>
  );
}

export function WorkflowsView({
  channels,
  editor,
  onCloseEditor,
  onCreateWorkflow,
  onDuplicateWorkflow,
  onEditWorkflow,
  onViewWorkflow,
  onEditorPaneChange,
}: WorkflowsViewProps) {
  const [deleteTarget, setDeleteTarget] = React.useState<Workflow | null>(null);
  const queryClient = useQueryClient();

  const editorWorkflowId =
    editor && editor.mode !== "create" ? editor.workflowId : null;

  const memberChannels = channels.filter((c) => c.isMember);
  const channelIds = memberChannels.map((c) => c.id).sort();
  const channelIdKey = channelIds.join(",");

  const allWorkflowsQuery = useQuery({
    queryKey: allWorkflowsQueryKey(channelIdKey),
    queryFn: async () => {
      // Single batched relay query for all member channels, then group by the
      // channel_id each workflow carries — replaces the per-channel fanout.
      const channelNameById = new Map(
        memberChannels.map((channel) => [channel.id, channel.name]),
      );
      const workflows = await getChannelsWorkflows(channelIds);
      const results: WorkflowWithChannel[] = [];
      for (const workflow of workflows) {
        results.push({
          workflow,
          channelName: workflow.channelId
            ? (channelNameById.get(workflow.channelId) ?? "")
            : "",
        });
      }
      return results;
    },
    enabled: memberChannels.length > 0,
    ...workflowListFocusRefetchPolicy,
  });

  const allWorkflows = allWorkflowsQuery.data ?? [];

  const triggerMutation = useMutation({
    mutationFn: (workflowId: string) => triggerWorkflow(workflowId),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] === "workflow-runs",
      });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (workflowId: string) => deleteWorkflow(workflowId),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        predicate: (query) =>
          query.queryKey[0] === "workflows" ||
          query.queryKey[0] === "workflows-all",
      });
    },
  });

  const toggleEnabledMutation = useMutation({
    mutationFn: (workflow: Workflow) =>
      updateWorkflow(
        workflow.id,
        yamlStringify(
          withWorkflowEnabled(
            workflow.definition,
            !getWorkflowEnabled(workflow.definition),
          ),
        ),
        workflow.revision,
      ),
    onError: (error) => {
      toast.error("Couldn’t change workflow status", {
        description:
          error instanceof Error
            ? error.message
            : "The workflow was not changed. Try again.",
      });
    },
    onSuccess: (_data, workflow) => {
      void queryClient.invalidateQueries({
        queryKey: workflowQueryKey(workflow.id),
      });
      void queryClient.invalidateQueries({
        predicate: (query) =>
          query.queryKey[0] === "workflows" ||
          query.queryKey[0] === "workflows-all",
      });
    },
  });

  const triggerOne = triggerMutation.mutate;
  const handleTrigger = React.useCallback(
    (workflowId: string) => triggerOne(workflowId),
    [triggerOne],
  );

  const handleDelete = React.useCallback(
    (workflow: Workflow) => setDeleteTarget(workflow),
    [],
  );

  const deleteOne = deleteMutation.mutateAsync;
  const handleConfirmDelete = React.useCallback(
    async (workflow: Workflow) => {
      try {
        await deleteOne(workflow.id);
        setDeleteTarget(null);
        // Deleting the workflow the editor is pointed at would otherwise leave
        // that editor open on a workflow that no longer exists.
        if (workflow.id === editorWorkflowId) onCloseEditor();
      } catch {
        // React Query stores the error; keep the confirmation and editor open.
      }
    },
    [deleteOne, editorWorkflowId, onCloseEditor],
  );

  const handleView = React.useCallback(
    (workflow: Workflow) => onViewWorkflow(workflow.id),
    [onViewWorkflow],
  );

  const handleEdit = React.useCallback(
    (workflow: Workflow) => onEditWorkflow(workflow.id),
    [onEditWorkflow],
  );

  const handleDuplicate = React.useCallback(
    (workflow: Workflow) => onDuplicateWorkflow(workflow.id),
    [onDuplicateWorkflow],
  );

  const toggleEnabled = toggleEnabledMutation.mutate;
  const handleToggleEnabled = React.useCallback(
    (workflow: Workflow) => toggleEnabled(workflow),
    [toggleEnabled],
  );

  const editorWorkflowHint = allWorkflows.find(
    ({ workflow }) => workflow.id === editorWorkflowId,
  )?.workflow;

  return (
    <div
      className="relative flex min-h-0 flex-1 overflow-hidden"
      data-testid="workflows-view"
    >
      <div
        className="flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8"
        data-scroll-restoration-id="workflows-list"
      >
        <div className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]">
          <PageHeader
            action={
              <Button
                aria-label="Refresh workflows"
                disabled={allWorkflowsQuery.isFetching}
                onClick={() => void allWorkflowsQuery.refetch()}
                size="icon"
                variant="ghost"
              >
                <RefreshCw
                  className={`h-4 w-4 ${allWorkflowsQuery.isFetching ? "animate-spin" : ""}`}
                />
              </Button>
            }
            description="Automations that keep your community moving."
            title="Workflows"
          />

          {allWorkflowsQuery.isLoading ? (
            <WorkflowsListSkeleton />
          ) : allWorkflowsQuery.isError ? (
            <div className="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
              <p className="text-sm text-red-400">Failed to load workflows</p>
              <Button
                onClick={() => void allWorkflowsQuery.refetch()}
                size="sm"
                variant="outline"
              >
                Retry
              </Button>
            </div>
          ) : (
            <div className={WORKFLOW_CARD_GRID_CLASS}>
              <CreateWorkflowCard onClick={onCreateWorkflow} />
              {allWorkflows.map(({ workflow, channelName }) => (
                <WorkflowCard
                  channelName={channelName}
                  isTogglingEnabled={
                    toggleEnabledMutation.isPending &&
                    toggleEnabledMutation.variables?.id === workflow.id
                  }
                  key={workflow.id}
                  onDelete={handleDelete}
                  onDuplicate={handleDuplicate}
                  onEdit={handleEdit}
                  onToggleEnabled={handleToggleEnabled}
                  onTrigger={handleTrigger}
                  onView={handleView}
                  workflow={workflow}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      <WorkflowEditorHost
        channels={memberChannels}
        editor={editor}
        onClose={onCloseEditor}
        onDeleteWorkflow={handleDelete}
        onDuplicateWorkflow={onDuplicateWorkflow}
        onEditWorkflow={onEditWorkflow}
        onEditorPaneChange={onEditorPaneChange}
        onTriggerWorkflow={handleTrigger}
        workflowHint={editorWorkflowHint}
      />

      <WorkflowDeleteDialog
        error={
          deleteMutation.error instanceof Error
            ? deleteMutation.error.message
            : null
        }
        isPending={deleteMutation.isPending}
        onConfirm={handleConfirmDelete}
        onOpenChange={(open) => {
          if (!open) {
            deleteMutation.reset();
            setDeleteTarget(null);
          }
        }}
        open={deleteTarget !== null}
        workflow={deleteTarget}
      />
    </div>
  );
}
