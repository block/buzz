import type { Project, Repository } from "@/features/projects/hooks";
import { isValidProjectChannelId } from "@/features/projects/projectModels";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import type { ProjectEventTemplate } from "./projectCreation";

export type AddedRepositoryEventTemplates = {
  project: ProjectEventTemplate;
  repository: ProjectEventTemplate;
  repositoryAddress: string;
  repositoryDtag: string;
};

function repositoryDtagFromName(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function buildProjectReplacementTemplate({
  ownerPubkey,
  project,
  repositoryAddresses,
}: {
  ownerPubkey: string;
  project: Project;
  repositoryAddresses: string[];
}): ProjectEventTemplate {
  const normalizedOwner = ownerPubkey.trim().toLowerCase();
  if (normalizedOwner !== project.owner.toLowerCase()) {
    throw new Error("Only the project owner can add repositories.");
  }
  if (repositoryAddresses.length > 64) {
    throw new Error("A project cannot contain more than 64 repositories.");
  }
  if (new Set(repositoryAddresses).size !== repositoryAddresses.length) {
    throw new Error("A project cannot contain duplicate repositories.");
  }
  if (
    repositoryAddresses.some(
      (address) => !/^30617:[0-9a-f]{64}:.+$/i.test(address),
    )
  ) {
    throw new Error("Repository address is invalid.");
  }

  const tags: string[][] = [
    ["d", project.dtag],
    ["name", project.name],
  ];
  if (project.description) tags.push(["description", project.description]);
  if (project.projectChannelId) {
    tags.push(["buzz-channel", project.projectChannelId]);
  }
  if (project.visibility === "unlisted") {
    tags.push(["buzz-visibility", "unlisted"]);
  }
  for (const address of repositoryAddresses.sort()) {
    const relayHint = project.repositoryRelayHints?.[address];
    tags.push(relayHint ? ["a", address, relayHint] : ["a", address]);
  }
  return { kind: KIND_PROJECT_ANNOUNCEMENT, content: "", tags };
}

export function buildAttachedRepositoryProjectEventTemplate({
  ownerPubkey,
  project,
  repositoryAddress,
}: {
  ownerPubkey: string;
  project: Project;
  repositoryAddress: string;
}): ProjectEventTemplate {
  if (project.repositoryAddresses.includes(repositoryAddress)) {
    throw new Error("This repository is already part of the project.");
  }
  return buildProjectReplacementTemplate({
    ownerPubkey,
    project,
    repositoryAddresses: [...project.repositoryAddresses, repositoryAddress],
  });
}

export function buildRepositoryChannelBindingTemplate({
  channelId,
  ownerPubkey,
  repository,
}: {
  channelId: string;
  ownerPubkey: string;
  repository: Repository;
}): ProjectEventTemplate {
  const normalizedChannelId = channelId.trim();
  if (ownerPubkey.trim().toLowerCase() !== repository.owner.toLowerCase()) {
    throw new Error("Only the repository owner can repair its access.");
  }
  if (!isValidProjectChannelId(normalizedChannelId)) {
    throw new Error("Repository access channel is invalid.");
  }
  if (!repository.eventTags) {
    throw new Error(
      "Repository metadata is unavailable. Refresh and try again.",
    );
  }

  return {
    kind: KIND_REPO_ANNOUNCEMENT,
    content: repository.eventContent ?? repository.description,
    tags: [
      ...repository.eventTags
        .filter((tag) => tag[0] !== "buzz-channel")
        .map((tag) => [...tag]),
      ["buzz-channel", normalizedChannelId],
    ],
  };
}

export function buildAddedRepositoryEventTemplates({
  accessChannelId,
  cloneUrl,
  description,
  name,
  ownerPubkey,
  project,
  webUrl,
}: {
  accessChannelId?: string;
  cloneUrl?: string;
  description?: string;
  name: string;
  ownerPubkey: string;
  project: Project;
  webUrl?: string;
}): AddedRepositoryEventTemplates {
  const normalizedOwner = ownerPubkey.trim().toLowerCase();

  const normalizedName = name.trim();
  if (!normalizedName) throw new Error("Repository name is required.");
  const repositoryDtag = repositoryDtagFromName(normalizedName);
  if (!repositoryDtag) {
    throw new Error("Repository name must include letters or numbers.");
  }

  const repositoryAddress = `${KIND_REPO_ANNOUNCEMENT}:${normalizedOwner}:${repositoryDtag}`;
  const isUnavailableMember =
    project.unavailableRepositoryAddresses?.includes(repositoryAddress) ??
    false;
  if (
    project.repositoryAddresses.includes(repositoryAddress) &&
    !isUnavailableMember
  ) {
    throw new Error(`This project already contains "${repositoryDtag}".`);
  }
  const normalizedDescription = description?.trim() ?? "";
  const repositoryTags: string[][] = [
    ["d", repositoryDtag],
    ["name", normalizedName],
  ];
  const normalizedAccessChannelId = accessChannelId?.trim();
  if (!normalizedAccessChannelId) {
    throw new Error(
      "This project has no repository access channel to inherit.",
    );
  }
  if (!isValidProjectChannelId(normalizedAccessChannelId)) {
    throw new Error("Repository access channel is invalid.");
  }
  repositoryTags.push(["buzz-channel", normalizedAccessChannelId]);
  if (normalizedDescription) {
    repositoryTags.push(["description", normalizedDescription]);
  }
  const normalizedCloneUrl = cloneUrl?.trim();
  if (normalizedCloneUrl) repositoryTags.push(["clone", normalizedCloneUrl]);
  const normalizedWebUrl = webUrl?.trim();
  if (normalizedWebUrl) repositoryTags.push(["web", normalizedWebUrl]);

  const projectTemplate = buildProjectReplacementTemplate({
    ownerPubkey,
    project,
    repositoryAddresses: isUnavailableMember
      ? [...project.repositoryAddresses]
      : [...project.repositoryAddresses, repositoryAddress],
  });

  return {
    project: projectTemplate,
    repository: {
      kind: KIND_REPO_ANNOUNCEMENT,
      content: normalizedDescription,
      tags: repositoryTags,
    },
    repositoryAddress,
    repositoryDtag,
  };
}
