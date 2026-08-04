import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import { addRepositoryToProject } from "@/features/projects/projectModels";
import { buildAttachedRepositoryProjectEventTemplate } from "@/features/projects/projectRepositoryCreation";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";

export type AttachProjectRepositoryInput = {
  project: Project;
  repository: Repository;
};

async function attachProjectRepository({
  project,
  repository,
}: AttachProjectRepositoryInput) {
  const identity = await getIdentity();
  const template = buildAttachedRepositoryProjectEventTemplate({
    ownerPubkey: identity.pubkey,
    project,
    repositoryAddress: repository.repoAddress,
  });
  const projectEvent = await signRelayEvent({
    ...template,
    createdAt: Math.max(Math.floor(Date.now() / 1_000), project.createdAt + 1),
  });
  await relayClient.publishEvent(
    projectEvent,
    "Timed out updating the project.",
    "Failed to attach the repository.",
  );

  return {
    previousProjectId: project.id,
    project: addRepositoryToProject(
      project,
      repository,
      projectEvent.created_at,
    ),
    repository,
  };
}

export function useAttachProjectRepositoryMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: attachProjectRepository,
    onSuccess: ({ previousProjectId, project }) => {
      queryClient.setQueryData(["project", project.id], project);
      if (previousProjectId !== project.id) {
        queryClient.removeQueries({
          exact: true,
          queryKey: ["project", previousProjectId],
        });
      }
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.map((candidate) =>
          candidate.id === previousProjectId ? project : candidate,
        ),
      );
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
    },
  });
}
