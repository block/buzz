import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchPlans,
  publishMissionConstraint,
  publishPlanningPlaybook,
  publishPlanningProject,
  publishPlanningTaskArtifact,
  publishPlanningTaskDetails,
  publishPlanningTask,
  publishPlanningTaskExecution,
} from "./data/plansService";
import type {
  MissionConstraint,
  PlanningProject,
  PlanningTask,
} from "./domain/contracts";
import type {
  PlanningPlaybookV1,
  PlanningTaskArtifactV1,
  PlanningTaskDetailsV1,
  PlanningTaskExecutionV1,
} from "./domain/extendedContracts";

export const plansQueryKey = (pubkey: string) => ["plans", pubkey] as const;
export function usePlansQuery(pubkey: string | undefined) {
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: plansQueryKey(pubkey ?? ""),
    queryFn: () => fetchPlans(pubkey ?? ""),
    staleTime: 30_000,
  });
}
export function usePlanMutations(pubkey: string) {
  const client = useQueryClient();
  const invalidate = () =>
    client.invalidateQueries({ queryKey: plansQueryKey(pubkey) });
  return {
    project: useMutation({
      mutationFn: (input: PlanningProject) =>
        publishPlanningProject(pubkey, input),
      onSuccess: invalidate,
    }),
    task: useMutation({
      mutationFn: (input: PlanningTask) => publishPlanningTask(pubkey, input),
      onSuccess: invalidate,
    }),
    constraint: useMutation({
      mutationFn: (input: MissionConstraint) =>
        publishMissionConstraint(pubkey, input),
      onSuccess: invalidate,
    }),
    details: useMutation({
      mutationFn: (input: PlanningTaskDetailsV1) =>
        publishPlanningTaskDetails(pubkey, input),
      onSuccess: invalidate,
    }),
    playbook: useMutation({
      mutationFn: (input: PlanningPlaybookV1) =>
        publishPlanningPlaybook(pubkey, input),
      onSuccess: invalidate,
    }),
    execution: useMutation({
      mutationFn: (input: PlanningTaskExecutionV1) =>
        publishPlanningTaskExecution(pubkey, input),
      onSuccess: invalidate,
    }),
    artifact: useMutation({
      mutationFn: (input: PlanningTaskArtifactV1) =>
        publishPlanningTaskArtifact(pubkey, input),
      onSuccess: invalidate,
    }),
  };
}
