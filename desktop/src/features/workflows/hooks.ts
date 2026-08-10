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
    staleTime: 30_000,
    refetchOnWindowFocus: true,
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
    staleTime: 10_000,
    refetchInterval: (query) => {
      if (!appFocused) return false;
      const runs = query.state.data as WorkflowRun[] | undefined;
      return runs?.some((run) => isActiveWorkflowRunStatus(run.status))
        ? 1_000
        : false;
    },
    refetchOnWindowFocus: true,
  });
}

export function useRunApprovalsQuery(
  workflowId: string | null,
  runId: string | null,
) {
  const refetchInterval = useFocusedRefetchInterval(10_000);

  return useQuery({
    queryKey: runApprovalsQueryKey(workflowId ?? "", runId ?? ""),
    queryFn: ({ queryKey: [, resolvedWorkflowId, resolvedRunId] }) =>
      getRunApprovals(resolvedWorkflowId, resolvedRunId),
    enabled: workflowId !== null && runId !== null,
    staleTime: 10_000,
    refetchInterval,
    refetchOnWindowFocus: true,
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
