import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import { addRepositoryToProject } from "@/features/projects/projectModels";
import { buildProjectPatchTemplate } from "@/features/projects/projectRepositoryCreation";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { KIND_PROJECT_ANNOUNCEMENT } from "@/shared/constants/kinds";

export type AttachProjectRepositoryInput = {
  project: Project;
  repository: Repository;
};

async function attachProjectRepository({
  project,
  repository,
}: AttachProjectRepositoryInput) {
  const identity = await getIdentity();

  // Fetch the live signed project head immediately before mutating.
  const liveHeads = await relayClient.fetchEvents({
    kinds: [KIND_PROJECT_ANNOUNCEMENT],
    authors: [identity.pubkey],
    "#d": [project.dtag],
    limit: 1,
  });
  const liveHead = liveHeads[0];
  if (!liveHead) {
    throw new Error(
      "Could not find this project on the relay. Refresh and try again.",
    );
  }

  // Dominated-write guard.
  if (liveHead.created_at > project.createdAt) {
    throw new Error(
      "This project was updated by another session while you were working. Refresh and try again.",
    );
  }

  const liveAddresses = liveHead.tags
    .filter((tag) => tag[0] === "a" && tag[1])
    .map((tag) => tag[1] as string);

  if (liveAddresses.includes(repository.repoAddress)) {
    throw new Error("This repository is already part of the project.");
  }
  const template = buildProjectPatchTemplate({
    liveHead,
    ownerPubkey: identity.pubkey,
    repositoryAddresses: [...liveAddresses, repository.repoAddress],
  });
  const projectEvent = await signRelayEvent({
    ...template,
    createdAt: Math.max(
      Math.floor(Date.now() / 1_000),
      liveHead.created_at + 1,
    ),
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
