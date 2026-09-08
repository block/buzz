import {
  createChannelManagedAgents,
  type CreateChannelManagedAgentInput,
} from "@/features/agents/channelAgents";
import { fetchProjects, type Project } from "@/features/projects/hooks";
import {
  buildDefaultProjectRepositoryTemplate,
  buildProjectBootstrapTemplates,
  conflictingListedProject,
  isUnsupportedProjectKindError,
  projectDtagFromName,
  type ProjectListingVisibility,
} from "@/features/projects/projectCreation";
import { buildProjectPatchTemplate } from "@/features/projects/projectRepositoryCreation";
import { buildProjectReadModels } from "@/features/projects/projectModels";
import { relayClient } from "@/shared/api/relayClient";
import { createChannel, signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type {
  Channel,
  ChannelVisibility,
  RelayEvent,
} from "@/shared/api/types";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";

export type CreateProjectInput = {
  name: string;
  description?: string;
  channelVisibility?: ChannelVisibility;
  projectVisibility?: ProjectListingVisibility;
  agents?: readonly CreateChannelManagedAgentInput[];
  templateId?: string;
};

export type CreateProjectResult = {
  channel: Channel | null;
  /** Retained for callers shared with legacy repository-only creation. */
  compatibilityWarning?: string;
  project: Project;
};

export type CreateProjectResumeState = {
  channels: Map<string, Channel>;
  projectIds: Set<string>;
};

function formatAgentFailures(
  failures: ReadonlyArray<{ name: string; error: string }>,
) {
  if (failures.length === 1) {
    const [failure] = failures;
    return `The project was created, but adding ${failure.name} failed: ${failure.error}`;
  }
  return `The project was created, but adding agents failed: ${failures
    .map((failure) => `${failure.name}: ${failure.error}`)
    .join("; ")}`;
}

async function publishProjectEvent(event: RelayEvent) {
  try {
    await relayClient.publishEvent(
      event,
      "Timed out creating project.",
      "Failed to create project.",
    );
  } catch (error) {
    if (isUnsupportedProjectKindError(error)) {
      throw new Error(
        "This relay does not support projects yet, so a project channel cannot be published here.",
      );
    }
    throw error;
  }
}

async function publishRepositoryEvent(event: RelayEvent) {
  await relayClient.publishEvent(
    event,
    "Timed out creating the project repository.",
    "Failed to create the project repository.",
  );
}

function readCreatedProject(
  projectEvent: RelayEvent,
  repositoryEvent: RelayEvent | null,
): Project {
  const [project] = buildProjectReadModels({
    projectEvents: [projectEvent],
    repositoryEvents: repositoryEvent ? [repositoryEvent] : [],
    relayOrigin: getCachedRelayOrigin(),
  });
  if (!project) {
    throw new Error("The project was created but could not be read.");
  }
  return project;
}

async function addRequestedAgents(
  channelId: string,
  agents: readonly CreateChannelManagedAgentInput[] | undefined,
) {
  if (!agents || agents.length === 0) return;
  const result = await createChannelManagedAgents(channelId, agents);
  if (result.failures.length > 0) {
    throw new Error(formatAgentFailures(result.failures));
  }
}

async function fetchOwnHead(
  kind: number,
  ownerPubkey: string,
  dtag: string,
): Promise<RelayEvent | null> {
  const events = await relayClient.fetchEvents({
    kinds: [kind],
    authors: [ownerPubkey],
    "#d": [dtag],
    limit: 1,
  });
  return events[0] ?? null;
}

async function ensureDefaultRepository({
  channelId,
  input,
  ownerPubkey,
  project,
}: {
  channelId: string;
  input: CreateProjectInput;
  ownerPubkey: string;
  project: Project;
}): Promise<Project> {
  const repositoryTemplate = buildDefaultProjectRepositoryTemplate({
    description: input.description,
    name: input.name,
    ownerPubkey,
    projectChannelId: channelId,
  });
  const existingRepository =
    project.repositories.find(
      (repository) =>
        repository.repoAddress === repositoryTemplate.repositoryAddress,
    ) ?? null;
  if (existingRepository) return project;

  let repositoryEvent = await fetchOwnHead(
    KIND_REPO_ANNOUNCEMENT,
    ownerPubkey,
    repositoryTemplate.dtag,
  );
  if (!repositoryEvent) {
    repositoryEvent = await signRelayEvent(repositoryTemplate.repository);
    await publishRepositoryEvent(repositoryEvent);
  }

  if (
    project.repositoryAddresses.includes(repositoryTemplate.repositoryAddress)
  ) {
    const liveProject =
      (await fetchOwnHead(
        KIND_PROJECT_ANNOUNCEMENT,
        ownerPubkey,
        project.dtag,
      )) ?? null;
    if (!liveProject) {
      throw new Error("The project was created but could not be read.");
    }
    return readCreatedProject(liveProject, repositoryEvent);
  }

  const liveHead = await fetchOwnHead(
    KIND_PROJECT_ANNOUNCEMENT,
    ownerPubkey,
    project.dtag,
  );
  if (!liveHead) {
    throw new Error(
      "Could not find this project on the relay. Refresh and try again.",
    );
  }
  const patched = buildProjectPatchTemplate({
    liveHead,
    ownerPubkey,
    repositoryAddresses: [
      ...new Set([
        ...project.repositoryAddresses,
        repositoryTemplate.repositoryAddress,
      ]),
    ].sort(),
  });
  const projectEvent = await signRelayEvent(patched);
  await publishProjectEvent(projectEvent);
  return readCreatedProject(projectEvent, repositoryEvent);
}

async function finishCreate(
  channel: Channel | null,
  project: Project,
  input: CreateProjectInput,
  resume: CreateProjectResumeState,
  projectId: string,
): Promise<CreateProjectResult> {
  const agentChannelId = channel?.id ?? project.projectChannelId;
  if (agentChannelId && input.agents && input.agents.length > 0) {
    await addRequestedAgents(agentChannelId, input.agents);
  }
  resume.projectIds.delete(projectId);
  resume.channels.delete(projectId);
  return { channel, project };
}

/** Creates the home channel, a bound default repository, and the NIP-MP project. */
export async function createProject(
  input: CreateProjectInput,
  resume: CreateProjectResumeState,
): Promise<CreateProjectResult> {
  const identity = await getIdentity();
  const dtagPreview = projectDtagFromName(input.name);
  if (!dtagPreview) {
    throw new Error("Project name must include letters or numbers.");
  }
  const existing = await fetchProjects();
  const ownerPubkey = identity.pubkey.toLowerCase();
  const existingProject = existing.find(
    (project) =>
      project.owner.toLowerCase() === ownerPubkey &&
      project.dtag === dtagPreview,
  );
  const projectId = `${ownerPubkey}:${dtagPreview}`;
  const canResume = resume.projectIds.has(projectId);
  if (existingProject && !canResume) {
    throw new Error(`You already have a project named "${dtagPreview}".`);
  }
  if (existingProject && !existingProject.legacy) {
    const cachedChannel = resume.channels.get(projectId) ?? null;
    const channelId =
      cachedChannel?.id ?? existingProject.projectChannelId ?? "";
    const project = channelId
      ? await ensureDefaultRepository({
          channelId,
          input,
          ownerPubkey,
          project: existingProject,
        })
      : existingProject;
    return finishCreate(cachedChannel, project, input, resume, projectId);
  }
  const conflict = conflictingListedProject(existing, {
    dtag: dtagPreview,
    name: input.name,
    ownerPubkey,
  });
  if (conflict) {
    throw new Error(
      `A project named "${conflict.name}" already exists. Open that one instead of creating another.`,
    );
  }

  resume.projectIds.add(projectId);
  let channel = resume.channels.get(projectId);
  if (!channel) {
    channel = await createChannel({
      channelType: "stream",
      description: input.description,
      name: input.name.trim(),
      visibility: input.channelVisibility ?? "open",
    });
    resume.channels.set(projectId, channel);
  }

  const templates = buildProjectBootstrapTemplates({
    description: input.description,
    name: input.name,
    ownerPubkey: identity.pubkey,
    projectChannelId: channel.id,
    projectVisibility: input.projectVisibility ?? "listed",
  });
  const existingRepositoryEvent = await fetchOwnHead(
    KIND_REPO_ANNOUNCEMENT,
    ownerPubkey,
    templates.dtag,
  );
  const projectEvent = await signRelayEvent(templates.project);
  await publishProjectEvent(projectEvent);

  let repositoryEvent = existingRepositoryEvent;
  if (!repositoryEvent) {
    repositoryEvent = await signRelayEvent(templates.repository);
    await publishRepositoryEvent(repositoryEvent);
  }

  const project = readCreatedProject(projectEvent, repositoryEvent);
  return finishCreate(channel, project, input, resume, projectId);
}
