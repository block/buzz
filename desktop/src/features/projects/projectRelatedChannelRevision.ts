import type { Project } from "@/features/projects/projectModels";
import { isValidProjectChannelId } from "@/features/projects/projectModels";
import { publishOwnedAgentProjectAnnouncements } from "@/features/projects/projectOwnerControl";
import { publishProjectOwnerAnnouncement } from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_PROJECT_REVISION } from "@/shared/constants/kinds";

export type ProjectRelatedChannelOperation =
  | "add-related-channel"
  | "remove-related-channel";

/** A signed revision whose relay publication outcome must be reconciled. */
export class ProjectRevisionPublicationError extends Error {
  readonly event: RelayEvent;
  readonly publicationError: unknown;

  constructor(event: RelayEvent, publicationError: unknown) {
    super(
      publicationError instanceof Error
        ? publicationError.message
        : "Could not publish the Project channel change.",
    );
    this.name = "ProjectRevisionPublicationError";
    this.event = event;
    this.publicationError = publicationError;
  }
}

export function buildProjectRelatedChannelRevisionTemplate(
  project: Pick<
    Project,
    | "effectiveRevisionId"
    | "baseRevisionId"
    | "legacy"
    | "projectAddress"
    | "projectChannelId"
    | "relatedChannelIds"
  >,
  channelId: string,
  operation: ProjectRelatedChannelOperation,
) {
  if (project.legacy || !project.projectAddress.startsWith("30621:")) {
    throw new Error("Only explicit Projects can link related channels.");
  }
  const expectedRevision = project.effectiveRevisionId?.toLowerCase();
  const baseRevision = project.baseRevisionId?.toLowerCase();
  if (!expectedRevision || !/^[0-9a-f]{64}$/.test(expectedRevision)) {
    throw new Error("Refresh this Project before changing its channels.");
  }
  if (!baseRevision || !/^[0-9a-f]{64}$/.test(baseRevision)) {
    throw new Error("Refresh this Project before changing its channels.");
  }
  const normalizedChannelId = channelId.trim().toLowerCase();
  if (!isValidProjectChannelId(normalizedChannelId)) {
    throw new Error("Project channel is invalid.");
  }
  if (normalizedChannelId === project.projectChannelId?.toLowerCase()) {
    throw new Error("The Project home channel cannot also be related.");
  }
  const alreadyRelated = project.relatedChannelIds.some(
    (candidate) => candidate.toLowerCase() === normalizedChannelId,
  );
  if (operation === "add-related-channel" && alreadyRelated) {
    throw new Error("That channel is already related to this Project.");
  }
  if (operation === "remove-related-channel" && !alreadyRelated) {
    throw new Error("That channel is not related to this Project.");
  }
  const relatedChannelIds =
    operation === "add-related-channel"
      ? [...project.relatedChannelIds, normalizedChannelId]
      : project.relatedChannelIds.filter(
          (candidate) => candidate.toLowerCase() !== normalizedChannelId,
        );
  return {
    kind: KIND_PROJECT_REVISION,
    content: "",
    tags: [
      ["a", project.projectAddress],
      ["base", baseRevision],
      ["e", expectedRevision],
      ["op", operation],
      ["channel", normalizedChannelId],
      ...relatedChannelIds
        .map((candidate) => candidate.toLowerCase())
        .sort()
        .map((candidate) => ["buzz-related-channel", candidate]),
    ],
  };
}

export async function publishProjectRelatedChannelRevision(
  project: Parameters<typeof buildProjectRelatedChannelRevisionTemplate>[0],
  channelId: string,
  operation: ProjectRelatedChannelOperation,
  deps?: {
    publishEvent?: typeof relayClient.publishEvent;
    publishOwnedAgentAnnouncements?: typeof publishOwnedAgentProjectAnnouncements;
    publishOwnerAnnouncement?: typeof publishProjectOwnerAnnouncement;
    signEvent?: typeof signRelayEvent;
  },
  signer?: {
    ownerControlAgentPubkey?: string;
    signAsManagedOwner?: boolean;
  },
): Promise<RelayEvent> {
  const template = buildProjectRelatedChannelRevisionTemplate(
    project,
    channelId,
    operation,
  );
  if (signer?.ownerControlAgentPubkey) {
    const [event] = await (
      deps?.publishOwnedAgentAnnouncements ??
      publishOwnedAgentProjectAnnouncements
    )(signer.ownerControlAgentPubkey, [template]);
    if (!event) {
      throw new Error("The Project owner agent returned no revision event.");
    }
    return event;
  }
  if (signer?.signAsManagedOwner) {
    const result = await (
      deps?.publishOwnerAnnouncement ?? publishProjectOwnerAnnouncement
    )({
      targetOwner: project.projectAddress.split(":")[1] ?? "",
      ...template,
    });
    if (result.publicationError) {
      throw new Error(result.publicationError);
    }
    return result.event;
  }
  const event = await (deps?.signEvent ?? signRelayEvent)(template);
  try {
    await (deps?.publishEvent ?? relayClient.publishEvent.bind(relayClient))(
      event,
      "Could not confirm the Project channel change. Refresh before retrying.",
      "Could not change the Project's related channels.",
    );
  } catch (error) {
    throw new ProjectRevisionPublicationError(event, error);
  }
  return event;
}

export async function removeProjectRelatedChannel(
  project: Project,
  channelId: string,
  deps?: Parameters<typeof publishProjectRelatedChannelRevision>[3],
  signer?: Parameters<typeof publishProjectRelatedChannelRevision>[4],
): Promise<Project> {
  const revision = await publishProjectRelatedChannelRevision(
    project,
    channelId,
    "remove-related-channel",
    deps,
    signer,
  );
  return {
    ...project,
    createdAt: Math.max(project.createdAt, revision.created_at),
    effectiveRevisionId: revision.id.toLowerCase(),
    relatedChannelIds: project.relatedChannelIds.filter(
      (candidate) => candidate.toLowerCase() !== channelId.toLowerCase(),
    ),
  };
}
