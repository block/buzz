import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import { isUnsupportedProjectKindError } from "@/features/projects/projectCreation";
import { buildAddedRepositoryEventTemplates } from "@/features/projects/projectRepositoryCreation";
import {
  addRepositoryToProject,
  eventToRepository,
} from "@/features/projects/projectModels";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { KIND_PROJECT_ANNOUNCEMENT } from "@/shared/constants/kinds";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";

export type AddProjectRepositoryInput = {
  accessChannelId?: string;
  cloneUrl?: string;
  description?: string;
  name: string;
  project: Project;
  webUrl?: string;
};

async function addProjectRepository({
  project,
  ...input
}: AddProjectRepositoryInput): Promise<{
  previousProjectId: string;
  project: Project;
  repository: Repository;
}> {
  const identity = await getIdentity();
  const templates = buildAddedRepositoryEventTemplates({
    ...input,
    ownerPubkey: identity.pubkey,
    project,
  });
  const [projectEvent, repositoryEvent] = await Promise.all([
    signRelayEvent({
      ...templates.project,
      createdAt: Math.max(
        Math.floor(Date.now() / 1_000),
        project.createdAt + 1,
      ),
    }),
    signRelayEvent(templates.repository),
  ]);

  try {
    // Confirm grouping support before publishing the repository so older
    // relays cannot leave a new standalone repository behind.
    await relayClient.publishEvent(
      projectEvent,
      "Timed out updating the project.",
      "Failed to update the project.",
    );
  } catch (error) {
    if (isUnsupportedProjectKindError(error)) {
      throw new Error(
        `This relay does not support multi-repository projects (event kind ${KIND_PROJECT_ANNOUNCEMENT}).`,
      );
    }
    throw error;
  }

  await relayClient.publishEvent(
    repositoryEvent,
    "Timed out creating the repository.",
    "Failed to create the repository.",
  );
  const repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
  if (!repository) {
    throw new Error("The repository was created but could not be read.");
  }

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

export function useAddProjectRepositoryMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: addProjectRepository,
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
