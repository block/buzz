import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  createProjectConnection,
  deleteProjectConnection,
  listProjectConnections,
  testProjectConnection,
  type ProjectConnectionDraft,
  updateProjectConnection,
} from "@/shared/api/tauriProjectConnections";
import type { ProjectConnectionScope } from "@/shared/api/projectConnectionTypes";

export const projectConnectionsQueryKey = (
  projectScope: ProjectConnectionScope | null,
) =>
  [
    "project-connections",
    projectScope?.relayUrl ?? "",
    projectScope?.operatorPubkey ?? "",
    projectScope?.projectAddress ?? "",
  ] as const;

export function useProjectConnectionsQuery(
  projectScope: ProjectConnectionScope | null,
  options?: { enabled?: boolean },
) {
  return useQuery({
    enabled: Boolean(projectScope) && (options?.enabled ?? true),
    queryKey: projectConnectionsQueryKey(projectScope),
    queryFn: () =>
      listProjectConnections(projectScope as ProjectConnectionScope),
  });
}

export function useCreateProjectConnectionMutation(
  projectScope: ProjectConnectionScope,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ProjectConnectionDraft) =>
      createProjectConnection(input),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: projectConnectionsQueryKey(projectScope),
      });
    },
  });
}

export function useUpdateProjectConnectionMutation(
  projectScope: ProjectConnectionScope,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ProjectConnectionDraft & { id: string }) =>
      updateProjectConnection(input),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: projectConnectionsQueryKey(projectScope),
      });
    },
  });
}

export function useTestProjectConnectionMutation(
  projectScope: ProjectConnectionScope,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (connectionId: string) =>
      testProjectConnection(projectScope, connectionId),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: projectConnectionsQueryKey(projectScope),
      });
    },
  });
}

export function useDeleteProjectConnectionMutation(
  projectScope: ProjectConnectionScope,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (connectionId: string) =>
      deleteProjectConnection(projectScope, connectionId),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: projectConnectionsQueryKey(projectScope),
      });
    },
  });
}
