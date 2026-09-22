import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useLocation } from "@tanstack/react-router";

import { useChannelsQuery } from "@/features/channels/hooks";
import { WorkflowDeleteDialog } from "@/features/workflows/ui/WorkflowDeleteDialog";
import {
  WorkflowEditorHost,
  type WorkflowEditorTarget,
} from "@/features/workflows/ui/WorkflowEditorHost";
import type { WorkflowEditorPane } from "@/features/workflows/ui/workflowEditorPane";
import { deleteWorkflow, triggerWorkflow } from "@/shared/api/tauriWorkflows";
import type { Workflow } from "@/shared/api/types";
import { WorkflowEditorOverlayProvider } from "@/shared/context/WorkflowEditorOverlayContext";

const INITIAL_PANE: WorkflowEditorPane = { type: "trigger" };

/** Rebuilds a target with a new pane without widening its discriminant. */
function withPane(
  target: WorkflowEditorTarget,
  pane: WorkflowEditorPane,
): WorkflowEditorTarget {
  return target.mode === "create"
    ? { initialChannelId: target.initialChannelId, mode: "create", pane }
    : { mode: target.mode, pane, workflowId: target.workflowId };
}

/**
 * Hosts the shared workflow editor as an overlay owned by the app shell, so
 * surfaces like channel settings can open a workflow without navigating away
 * from the channel. The Workflows route keeps its own URL-addressable host —
 * both render the same editor.
 */
export function AppWorkflowEditorOverlayProvider({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  const queryClient = useQueryClient();
  const channelsQuery = useChannelsQuery();
  const memberChannels = React.useMemo(
    () => (channelsQuery.data ?? []).filter((channel) => channel.isMember),
    [channelsQuery.data],
  );

  const [editor, setEditor] = React.useState<WorkflowEditorTarget | null>(null);
  const [workflowHint, setWorkflowHint] = React.useState<Workflow | undefined>(
    undefined,
  );
  const [deleteTarget, setDeleteTarget] = React.useState<Workflow | null>(null);

  const handleOpenWorkflow = React.useCallback(
    (workflowId: string, workflow?: Workflow) => {
      setWorkflowHint(workflow);
      setEditor({ mode: "detail", pane: INITIAL_PANE, workflowId });
    },
    [],
  );

  const handleOpenNewWorkflow = React.useCallback((channelId?: string) => {
    setWorkflowHint(undefined);
    setEditor({
      initialChannelId: channelId,
      mode: "create",
      pane: INITIAL_PANE,
    });
  }, []);

  const closeEditor = React.useCallback(() => {
    setEditor(null);
    setWorkflowHint(undefined);
  }, []);

  // This editor belongs to the surface that opened it. If the route leaves that
  // surface anyway, drop it rather than trailing the modal onto the next screen.
  // The editor's own dirty-exit guard runs first, so unsaved work still prompts.
  const { pathname } = useLocation();
  const lastPathnameRef = React.useRef(pathname);
  React.useEffect(() => {
    if (lastPathnameRef.current === pathname) return;
    lastPathnameRef.current = pathname;
    closeEditor();
  }, [closeEditor, pathname]);

  const handleEditorPaneChange = React.useCallback(
    (pane: WorkflowEditorPane) => {
      setEditor((current) => (current ? withPane(current, pane) : current));
    },
    [],
  );

  const handleEditWorkflow = React.useCallback((workflowId: string) => {
    setEditor({ mode: "edit", pane: INITIAL_PANE, workflowId });
  }, []);

  const handleDuplicateWorkflow = React.useCallback((workflowId: string) => {
    setEditor({ mode: "duplicate", pane: INITIAL_PANE, workflowId });
  }, []);

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

  const triggerOne = triggerMutation.mutate;
  const handleTriggerWorkflow = React.useCallback(
    (workflowId: string) => triggerOne(workflowId),
    [triggerOne],
  );

  const deleteOne = deleteMutation.mutateAsync;
  const handleConfirmDelete = React.useCallback(
    async (workflow: Workflow) => {
      try {
        await deleteOne(workflow.id);
        setDeleteTarget(null);
        closeEditor();
      } catch {
        // React Query stores the error; keep the confirmation and editor open.
      }
    },
    [closeEditor, deleteOne],
  );

  return (
    <WorkflowEditorOverlayProvider
      onOpenNewWorkflow={handleOpenNewWorkflow}
      onOpenWorkflow={handleOpenWorkflow}
    >
      {children}
      <WorkflowEditorHost
        channels={memberChannels}
        editor={editor}
        onClose={closeEditor}
        onDeleteWorkflow={setDeleteTarget}
        onDuplicateWorkflow={handleDuplicateWorkflow}
        onEditWorkflow={handleEditWorkflow}
        onEditorPaneChange={handleEditorPaneChange}
        onTriggerWorkflow={handleTriggerWorkflow}
        workflowHint={workflowHint}
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
    </WorkflowEditorOverlayProvider>
  );
}
