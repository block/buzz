import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import { isUnsupportedProjectKindError } from "@/features/projects/projectCreation";
import { buildAddedRepositoryEventTemplatesFromHead } from "@/features/projects/projectRepositoryCreation";
import {
  addRepositoryToProject,
  eventToRepository,
} from "@/features/projects/projectModels";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
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

  // Fetch the live signed project head immediately before mutating.
  // This prevents: (a) unknown-tag erasure from the cached UI projection,
  // (b) concurrent-write clobber when another session or the CLI advanced
  // the head after we loaded the page.
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

  // Dominated-write guard: if the live head is newer than our cached snapshot,
  // a concurrent session has already advanced the project — our mutation would
  // overwrite their changes. Surface the conflict rather than silently clobbering.
  if (liveHead.created_at > project.createdAt) {
    throw new Error(
      "This project was updated by another session while you were working. Refresh and try again.",
    );
  }

  const templates = buildAddedRepositoryEventTemplatesFromHead({
    ...input,
    existingRepositoryAddresses: project.repositoryAddresses,
    liveHead,
    ownerPubkey: identity.pubkey,
  });

  // Cross-project d-tag clobber guard: check if the owner already has a
  // different standalone or project-scoped 30617 at this coordinate.
  const existingRepoHeads = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    authors: [identity.pubkey],
    "#d": [templates.repositoryDtag],
    limit: 1,
  });
  if (existingRepoHeads.length > 0) {
    const existingAddress = `${KIND_REPO_ANNOUNCEMENT}:${identity.pubkey.toLowerCase()}:${templates.repositoryDtag}`;
    // If it's already in this project, the earlier duplicate-address check would
    // have thrown. If it's a different project or standalone repo, alert the user.
    if (
      !project.repositoryAddresses.includes(existingAddress) &&
      existingAddress !== templates.repositoryAddress
    ) {
      throw new Error(
        `A repository named "${templates.repositoryDtag}" already exists (as a standalone repository or in another project). Choose a different name to avoid overwriting it.`,
      );
    }
  }

  const [projectEvent, repositoryEvent] = await Promise.all([
    signRelayEvent({
      ...templates.project,
      createdAt: Math.max(
        Math.floor(Date.now() / 1_000),
        liveHead.created_at + 1,
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

  // If the repository publish fails, the project event is already live with a
  // dangling member. Persist the repository address so the caller can surface
  // the partial state (e.g., the "unavailable repository" affordance) and the
  // user can retry via the repair dialog.
  let repository: Repository | null = null;
  try {
    await relayClient.publishEvent(
      repositoryEvent,
      "Timed out creating the repository.",
      "Failed to create the repository.",
    );
    repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
    if (!repository) {
      throw new Error("The repository was created but could not be read.");
    }
  } catch (repoError) {
    // Surface a meaningful partial-write error. The project already lists the
    // repository coordinate; the user can see it as "unavailable" and retry.
    const message =
      repoError instanceof Error
        ? repoError.message
        : "Failed to create the repository.";
    throw new Error(
      `The project was updated but the repository could not be created: ${message} ` +
        `The incomplete repository will appear as unavailable — use the repair dialog to retry.`,
    );
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
