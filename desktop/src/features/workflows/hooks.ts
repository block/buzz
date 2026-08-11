import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { WorkflowRun, WorkflowRunStatus } from "@/shared/api/types";
import {
  useAppFocused,
  useFocusedRefetchInterval,
} from "@/shared/lib/useDocumentVisible";
import {
  createWorkflow,
  deleteWorkflow,
  denyApproval,
  getChannelWorkflows,
  getRunApprovals,
  getWorkflow,
  getWorkflowRuns,
  grantApproval,
  triggerWorkflow,
  updateWorkflow,
} from "@/shared/api/tauriWorkflows";

/** Stale gate for workflow list queries (useChannelWorkflowsQuery, allWorkflowsQuery).
 * These queries have no poll and no relay push subscription; invalidateWorkflowListQueries
 * only fires for mutations performed by this renderer, so remote workflow creates/edits/deletes
 * surface only via refetch on focus return.  10 s suppresses rapid focus-flip burst refetches
 * while keeping remote changes visible on the next focus return. */
export const WORKFLOW_LIST_FOCUS_STALE_TIME_MS = 10_000;
/** Keeps focused polling for run approvals at the established 10-second cadence. */
export const RUN_APPROVALS_REFETCH_INTERVAL_MS = 10_000;
/** Tighter stale gate for the workflow-runs detail query.
 * The refetchInterval is conditional — 1 s while any run is active, false
 * otherwise — so when there are no active runs the poll stops.  Remote-started
 * runs (webhook / another user) do not push an invalidation, meaning a 5-min
 * focus gate would leave the detail view stale for up to 5 minutes.  10 s
 * restores the pre-PR behavior and surfaces remote starts quickly without
 * excess load.  (Completed runs self-heal: the stale "active" cache keeps the
 * 1 s poll alive until the run finishes, so the fix only matters for the
 * "no active runs" → "new remote run" transition.) */
export const WORKFLOW_RUNS_FOCUS_STALE_TIME_MS = 10_000;
/** Suppresses redundant focus refetches for the run-approvals query.
 * The focused 10-second poll (RUN_APPROVALS_REFETCH_INTERVAL_MS) already keeps
 * this data fresh; a focus return within 5 minutes adds no new information. */
export const RUN_APPROVALS_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy for workflow list queries; consumed by focusRefetchPolicy.test.mjs. */
export const workflowListFocusRefetchPolicy = {
  staleTime: WORKFLOW_LIST_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: true,
} as const;

/** Focus-refetch policy for the workflow-runs query; consumed by focusRefetchPolicy.test.mjs. */
export const workflowRunsFocusRefetchPolicy = {
  staleTime: WORKFLOW_RUNS_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: true,
} as const;

/** Focus-refetch policy for the run-approvals query; consumed by focusRefetchPolicy.test.mjs. */
export const runApprovalsFocusRefetchPolicy = {
  staleTime: RUN_APPROVALS_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: true,
} as const;

export const allWorkflowsQueryKey = (channelIdKey: string) =>
  ["workflows-all", channelIdKey] as const;
export const workflowsQueryKey = (channelId: string) =>
  ["workflows", channelId] as const;
export const workflowQueryKey = (workflowId: string) =>
  ["workflow", workflowId] as const;
export const workflowRunsQueryKey = (workflowId: string) =>
  ["workflow-runs", workflowId] as const;
export const runApprovalsQueryKey = (workflowId: string, runId: string) =>
  ["run-approvals", workflowId, runId] as const;

function invalidateWorkflowListQueries(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  void queryClient.invalidateQueries({
    predicate: (query) =>
      query.queryKey[0] === "workflows" ||
      query.queryKey[0] === "workflows-all",
  });
}

function isActiveWorkflowRunStatus(status: WorkflowRunStatus) {
  return (
    status === "pending" ||
    status === "running" ||
    status === "waiting_approval"
  );
}

export function useChannelWorkflowsQuery(channelId: string | null) {
  return useQuery({
    queryKey: workflowsQueryKey(channelId ?? ""),
    queryFn: ({ queryKey: [, resolvedChannelId] }) =>
      getChannelWorkflows(resolvedChannelId),
    enabled: channelId !== null,
    ...workflowListFocusRefetchPolicy,
  });
}

export function useWorkflowQuery(workflowId: string | null) {
  return useQuery({
    queryKey: workflowQueryKey(workflowId ?? ""),
    queryFn: ({ queryKey: [, resolvedWorkflowId] }) =>
      getWorkflow(resolvedWorkflowId),
    enabled: workflowId !== null,
    staleTime: 30_000,
  });
}

export function useWorkflowRunsQuery(workflowId: string | null) {
  const appFocused = useAppFocused();
  return useQuery({
    queryKey: workflowRunsQueryKey(workflowId ?? ""),
    queryFn: ({ queryKey: [, resolvedWorkflowId] }) =>
      getWorkflowRuns(resolvedWorkflowId),
    enabled: workflowId !== null,
    refetchInterval: (query) => {
      if (!appFocused) return false;
      const runs = query.state.data as WorkflowRun[] | undefined;
      return runs?.some((run) => isActiveWorkflowRunStatus(run.status))
        ? 1_000
        : false;
    },
    ...workflowRunsFocusRefetchPolicy,
  });
}

export function useRunApprovalsQuery(
  workflowId: string | null,
  runId: string | null,
) {
  const refetchInterval = useFocusedRefetchInterval(
    RUN_APPROVALS_REFETCH_INTERVAL_MS,
  );

  return useQuery({
    queryKey: runApprovalsQueryKey(workflowId ?? "", runId ?? ""),
    queryFn: ({ queryKey: [, resolvedWorkflowId, resolvedRunId] }) =>
      getRunApprovals(resolvedWorkflowId, resolvedRunId),
    enabled: workflowId !== null && runId !== null,
    refetchInterval,
    ...runApprovalsFocusRefetchPolicy,
  });
}

export function useCreateWorkflowMutation(channelId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (yamlDefinition: string) =>
      createWorkflow(channelId, yamlDefinition),
    onSuccess: () => {
      invalidateWorkflowListQueries(queryClient);
    },
  });
}

export function useUpdateWorkflowMutation(workflowId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (yamlDefinition: string) =>
      updateWorkflow(workflowId, yamlDefinition),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: workflowQueryKey(workflowId),
      });
      invalidateWorkflowListQueries(queryClient);
    },
  });
}

export function useDeleteWorkflowMutation(workflowId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => deleteWorkflow(workflowId),
    onSuccess: () => {
      invalidateWorkflowListQueries(queryClient);
    },
  });
}

export function useTriggerWorkflowMutation(workflowId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => triggerWorkflow(workflowId),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: workflowRunsQueryKey(workflowId),
      });
    },
  });
}

export function useApprovalMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: {
      token: string;
      action: "grant" | "deny";
      note?: string;
    }) =>
      input.action === "grant"
        ? grantApproval(input.token, input.note)
        : denyApproval(input.token, input.note),
    onSuccess: (_data, _variables) => {
      void queryClient.invalidateQueries({
        predicate: (query) =>
          query.queryKey[0] === "workflow-runs" ||
          query.queryKey[0] === "workflow" ||
          query.queryKey[0] === "run-approvals",
      });
    },
  });
}
