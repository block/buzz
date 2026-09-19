import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  type Project,
  projectsQueryKey,
  type Repository,
} from "@/features/projects/hooks";
import { isUnsupportedProjectKindError } from "@/features/projects/projectCreation";
import {
  isDanglingProjectMemberPublish,
  publishOwnedAgentProjectAnnouncements,
} from "@/features/projects/projectOwnerControl";
import {
  buildAddedRepositoryEventTemplatesFromHead,
  repositoryDtagFromName,
} from "@/features/projects/projectRepositoryCreation";
import {
  addRepositoryToProject,
  eventToRepository,
} from "@/features/projects/projectModels";
import { markProjectDataAuthoritative } from "@/features/projects/projectSnapshot";
import { publishProjectOwnerAnnouncement } from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
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
  ownerControlAgentPubkey?: string;
  project: Project;
  webUrl?: string;
};

type FetchEventsInput = Parameters<(typeof relayClient)["fetchEvents"]>[0];

/**
 * Relay/publish seams injected by tests. The regression for the
 * partial-publish heal must run the mutation itself — guards included — not
 * just the template builder, so the whole flow takes its I/O through here.
 */
export type AddProjectRepositoryDeps = {
  fetchEvents: (filter: FetchEventsInput) => Promise<RelayEvent[]>;
  publishOwnedAgentAnnouncements: typeof publishOwnedAgentProjectAnnouncements;
  publishOwnerAnnouncement: typeof publishProjectOwnerAnnouncement;
};

/** Exported for tests — production callers go through the mutation hook. */
export async function addProjectRepository(
  { ownerControlAgentPubkey, project, ...input }: AddProjectRepositoryInput,
  deps?: Partial<AddProjectRepositoryDeps>,
): Promise<{
  previousProjectId: string;
  project: Project;
  repository: Repository;
}> {
  const {
    fetchEvents = relayClient.fetchEvents.bind(relayClient),
    publishOwnedAgentAnnouncements = publishOwnedAgentProjectAnnouncements,
    publishOwnerAnnouncement = publishProjectOwnerAnnouncement,
  } = deps ?? {};
  const targetOwner = project.owner.toLowerCase();

  // Fetch the live signed project head immediately before mutating.
  // This prevents: (a) unknown-tag erasure from the cached UI projection,
  // (b) concurrent-write clobber when another session or the CLI advanced
  // the head after we loaded the page.
  const liveHeads = await fetchEvents({
    kinds: [KIND_PROJECT_ANNOUNCEMENT],
    authors: [targetOwner],
    "#d": [project.dtag],
    limit: 1,
  });
  const liveHead = liveHeads[0];
  if (!liveHead) {
    throw new Error(
      "Could not find this project on the relay. Refresh and try again.",
    );
  }

  // Partial-publish detection must precede the dominated-write guard below:
  // the project event can land while the repository event fails, leaving the
  // live head referencing a coordinate with no repository head (a dangling
  // member) — and a live head strictly newer than the cached snapshot,
  // because the failed mutation never updated the cache. Probe for a
  // repository head at the coordinate first so the template builder can
  // distinguish "raced with another session" (throw) from "resume a partial
  // publish" (heal by publishing only the missing event).
  const repositoryDtag = repositoryDtagFromName(input.name.trim());
  const existingRepoHeads = repositoryDtag
    ? await fetchEvents({
        kinds: [KIND_REPO_ANNOUNCEMENT],
        authors: [targetOwner],
        "#d": [repositoryDtag],
        limit: 1,
      })
    : [];

  const templates = buildAddedRepositoryEventTemplatesFromHead({
    ...input,
    existingRepositoryAddresses: project.repositoryAddresses,
    liveHead,
    ownerPubkey: targetOwner,
    repositoryHeadExists: existingRepoHeads.length > 0,
  });

  // Cross-project d-tag clobber guard: if the owner already has a 30617
  // at this coordinate — whether as a standalone repo or in another project —
  // block the write unconditionally. (A coordinate that is already a member
  // of this project with a live repository head threw "already contains"
  // inside the template builder before reaching here.)
  if (existingRepoHeads.length > 0) {
    throw new Error(
      `A repository named "${templates.repositoryDtag}" already exists (as a standalone repository or in another project). Choose a different name to avoid overwriting it.`,
    );
  }

  // Dominated-write guard: if the live head is newer than our cached snapshot,
  // a concurrent session has already advanced the project — our mutation would
  // overwrite their changes. Surface the conflict rather than silently
  // clobbering. Resume mode publishes no project head, so a moved head cannot
  // be clobbered and must not block the heal.
  if (!templates.resume && liveHead.created_at > project.createdAt) {
    throw new Error(
      "This project was updated by another session while you were working. Refresh and try again.",
    );
  }

  const projectCreatedAt = Math.max(
    Math.floor(Date.now() / 1_000),
    liveHead.created_at + 1,
  );
  if (ownerControlAgentPubkey) {
    // Resume mode: the live project head already references the coordinate,
    // so republishing it would only advance created_at for nothing — send
    // just the missing repository event.
    let events: RelayEvent[];
    try {
      events = await publishOwnedAgentAnnouncements(
        ownerControlAgentPubkey,
        templates.resume
          ? [{ ...templates.repository, createdAt: projectCreatedAt }]
          : [
              { ...templates.project, createdAt: projectCreatedAt },
              { ...templates.repository, createdAt: projectCreatedAt },
            ],
      );
    } catch (error) {
      // Sequential publish failed partway: the project head landed but the
      // repository event did not. Consume the partial-success metadata and
      // resume immediately with just the missing event, instead of
      // surfacing a dangling member for the user to retry.
      if (!isDanglingProjectMemberPublish(error)) {
        throw error;
      }
      const repositoryEvents = await publishOwnedAgentAnnouncements(
        ownerControlAgentPubkey,
        [{ ...templates.repository, createdAt: projectCreatedAt }],
      );
      events = [...error.publishedEvents, ...repositoryEvents];
    }
    const projectEvent = templates.resume
      ? liveHead
      : events.find((event) => event.kind === KIND_PROJECT_ANNOUNCEMENT);
    const repositoryEvent = events.find(
      (event) => event.kind === KIND_REPO_ANNOUNCEMENT,
    );
    const repository = repositoryEvent
      ? eventToRepository(repositoryEvent, getCachedRelayOrigin())
      : null;
    if (!projectEvent || !repository) {
      throw new Error(
        "The owner agent updated the project but its response was incomplete. Refresh and try again.",
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

  let projectEvent: RelayEvent;
  if (templates.resume) {
    // The live head already references the coordinate — publishing the
    // missing repository event below is the entire heal.
    projectEvent = liveHead;
  } else {
    try {
      // Confirm grouping support before publishing the repository so older
      // relays cannot leave a new standalone repository behind.
      const publication = await publishOwnerAnnouncement({
        ...templates.project,
        createdAt: projectCreatedAt,
        targetOwner,
      });
      projectEvent = publication.event;
      if (publication.publicationError) {
        throw new Error(publication.publicationError);
      }
    } catch (error) {
      if (isUnsupportedProjectKindError(error)) {
        throw new Error(
          `This relay does not support multi-repository projects (event kind ${KIND_PROJECT_ANNOUNCEMENT}).`,
        );
      }
      throw error;
    }
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
  let repositoryEvent: RelayEvent | null = null;
  const publishRepository = async (): Promise<RelayEvent> => {
    const publication = await publishOwnerAnnouncement({
      ...templates.repository,
      createdAt: projectCreatedAt,
      targetOwner,
    });
    repositoryEvent = publication.event;
    if (publication.publicationError) {
      throw new Error(publication.publicationError);
    }
    return publication.event;
  };
  try {
    repositoryEvent = await publishRepository();
    repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
    if (!repository) {
      throw new Error("The repository was created but could not be read.");
    }
  } catch (_repoError) {
    // Query by event id: if the relay already has this event, the ACK was
    // merely lost — the write succeeded and we can proceed normally.
    let alreadyStored = false;
    if (repositoryEvent) {
      try {
        const stored = await fetchEvents({
          ids: [repositoryEvent.id],
          kinds: [KIND_REPO_ANNOUNCEMENT],
          limit: 1,
        });
        alreadyStored = stored.length > 0;
      } catch {
        // Ignore — if the query itself fails we fall through to the retry.
      }
    }
    if (alreadyStored && repositoryEvent) {
      repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
    } else {
      // The relay rejected or never received the event. Retry once with the
      // same signed event (safe — deterministic id).
      try {
        repositoryEvent = await publishRepository();
        repository = eventToRepository(repositoryEvent, getCachedRelayOrigin());
      } catch (retryError) {
        const message =
          retryError instanceof Error
            ? retryError.message
            : "Failed to create the repository.";
        // The project already lists the coordinate. Surface a partial-write
        // error with the event id so the user (or a future repair pass) can
        // query the relay directly and re-submit the signed event if needed.
        const eventLabel = repositoryEvent
          ? `event ${repositoryEvent.id.slice(0, 8)}…`
          : "an event that could not be signed";
        throw new Error(
          `The project was updated but the repository could not be created (${eventLabel}): ${message} ` +
            `The add-repository operation is idempotent — re-run Add repository with the same name to resume: the recovery query will find the stored event or retry the publish.`,
        );
      }
    }
    if (!repository && repositoryEvent) {
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
    mutationFn: (input: AddProjectRepositoryInput) =>
      addProjectRepository(input),
    onSuccess: ({ previousProjectId, project }) => {
      markProjectDataAuthoritative(project, "local-write");
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
