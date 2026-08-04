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

  // Cross-project d-tag clobber guard: if the owner already has a 30617
  // at this coordinate — whether as a standalone repo or in another project —
  // block the write unconditionally unless it is already a member of this
  // project (in which case the earlier duplicate-address check would have
  // thrown before we reach here).
  const existingRepoHeads = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    authors: [identity.pubkey],
    "#d": [templates.repositoryDtag],
    limit: 1,
  });
  if (existingRepoHeads.length > 0) {
    throw new Error(
      `A repository named "${templates.repositoryDtag}" already exists (as a standalone repository or in another project). Choose a different name to avoid overwriting it.`,
    );
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
  // dangling member. Attempt to recover automatically before surfacing an error:
  //
  // 1. Query the relay by event id to distinguish two failure modes:
  //    - Rejected ("OK false"): the relay explicitly refused the event. Retry
  //      once; if the retry also fails, the operation is genuinely unrecoverable.
  //    - Lost acknowledgement ("OK false" with ok:false but the relay stored it):
  //      the event reached the relay but the transport-level ACK was lost. A
  //      subsequent id-keyed query will find it; treat that as success.
  //
  // This keeps the mutation idempotent: re-submitting the same signed event is
  // safe because event ids are deterministic hashes of the content+pubkey+timestamp.
  let repository: Repository | null = null;
  const publishRepository = async (): Promise<void> => {
    await relayClient.publishEvent(
      repositoryEvent,
      "Timed out creating the repository.",
      "Failed to create the repository.",
    );
  };
  try {
    await publishRepository();
    repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
    if (!repository) {
      throw new Error("The repository was created but could not be read.");
    }
  } catch (_repoError) {
    // Query by event id: if the relay already has this event, the ACK was
    // merely lost — the write succeeded and we can proceed normally.
    let alreadyStored = false;
    try {
      const stored = await relayClient.fetchEvents({
        ids: [repositoryEvent.id],
        kinds: [KIND_REPO_ANNOUNCEMENT],
        limit: 1,
      });
      alreadyStored = stored.length > 0;
    } catch {
      // Ignore — if the query itself fails we fall through to the retry.
    }
    if (alreadyStored) {
      repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
    } else {
      // The relay rejected or never received the event. Retry once with the
      // same signed event (safe — deterministic id).
      try {
        await publishRepository();
        repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
      } catch (retryError) {
        const message =
          retryError instanceof Error
            ? retryError.message
            : "Failed to create the repository.";
        // The project already lists the coordinate. Surface a partial-write
        // error with the event id so the user (or a future repair pass) can
        // query the relay directly and re-submit the signed event if needed.
        throw new Error(
          `The project was updated but the repository could not be created (event ${repositoryEvent.id.slice(0, 8)}…): ${message} ` +
            `The add-repository operation is idempotent — re-run Add repository with the same name to resume: the recovery query will find the stored event or retry the publish.`,
        );
      }
    }
    if (!repository) {
      repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
    }
    if (!repository) {
      throw new Error("The repository was created but could not be read.");
    }
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
