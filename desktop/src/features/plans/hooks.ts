import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchPlans,
  publishMissionConstraint,
  publishPlanningProject,
  publishPlanningTask,
} from "./data/plansService";
import type {
  MissionConstraint,
  PlanningProject,
  PlanningTask,
} from "./domain/contracts";

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
  };
}
