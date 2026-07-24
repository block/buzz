import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import { effectiveCloneUrls } from "./lib/projectCloneUrl";

export type Repository = {
  id: string;
  dtag: string;
  name: string;
  description: string;
  cloneUrls: string[];
  webUrl: string | null;
  owner: string;
  contributors: string[];
  createdAt: number;
  status: string;
  defaultBranch: string;
  repoAddress: string;
};

export type Project = {
  id: string;
  dtag: string;
  name: string;
  description: string;
  owner: string;
  createdAt: number;
  projectChannelId: string | null;
  status: string;
  projectAddress: string;
  primaryRepositoryAddress: string | null;
  repositoryAddresses: string[];
  repositories: Repository[];
  legacy: boolean;
};

type BuildProjectReadModelsInput = {
  projectEvents: RelayEvent[];
  repositoryEvents: RelayEvent[];
  relayOrigin?: string | null;
};

function getTag(event: RelayEvent, name: string): string | undefined {
  const value = event.tags.find((tag) => tag[0] === name)?.[1];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function getAllTags(event: RelayEvent, name: string): string[] {
  return event.tags
    .filter(
      (tag) =>
        tag[0] === name && typeof tag[1] === "string" && tag[1].length > 0,
    )
    .map((tag) => tag[1]);
}

function getCloneUrls(event: RelayEvent): string[] {
  const tag = event.tags.find((candidate) => candidate[0] === "clone");
  return tag?.slice(1).filter((value) => value.length > 0) ?? [];
}

function isValidIdentifier(value: string): boolean {
  return (
    value.length > 0 &&
    value.length <= 64 &&
    !value.startsWith(".") &&
    !value.includes("..") &&
    /^[a-zA-Z0-9._-]+$/.test(value)
  );
}

function isValidPubkey(value: string): boolean {
  return /^[a-fA-F0-9]{64}$/.test(value);
}

function deduplicateAddressableEvents(events: RelayEvent[]): RelayEvent[] {
  const latest = new Map<string, RelayEvent>();
  for (const event of events) {
    const dtag = getTag(event, "d");
    if (!dtag) continue;
    const key = `${event.kind}:${event.pubkey.toLowerCase()}:${dtag}`;
    const current = latest.get(key);
    if (
      !current ||
      event.created_at > current.created_at ||
      (event.created_at === current.created_at && event.id < current.id)
    ) {
      latest.set(key, event);
    }
  }
  return [...latest.values()];
}

function parseRepositoryAddress(
  value: string,
): { owner: string; dtag: string } | null {
  const firstSeparator = value.indexOf(":");
  const secondSeparator = value.indexOf(":", firstSeparator + 1);
  if (
    value.slice(0, firstSeparator) !== String(KIND_REPO_ANNOUNCEMENT) ||
    secondSeparator < 0
  ) {
    return null;
  }

  const owner = value.slice(firstSeparator + 1, secondSeparator);
  const dtag = value.slice(secondSeparator + 1);
  return isValidPubkey(owner) && isValidIdentifier(dtag)
    ? { owner: owner.toLowerCase(), dtag }
    : null;
}

export function eventToRepository(
  event: RelayEvent,
  relayOrigin?: string | null,
): Repository | null {
  const dtag = getTag(event, "d");
  if (
    event.kind !== KIND_REPO_ANNOUNCEMENT ||
    !dtag ||
    !isValidIdentifier(dtag) ||
    !isValidPubkey(event.pubkey)
  ) {
    return null;
  }

  const owner = event.pubkey.toLowerCase();
  const setupUsers = getAllTags(event, "auth");
  return {
    id: `${owner}:${dtag}`,
    dtag,
    name: getTag(event, "name") ?? dtag,
    description: getTag(event, "description") ?? event.content ?? "",
    cloneUrls: effectiveCloneUrls(
      getCloneUrls(event),
      relayOrigin,
      owner,
      dtag,
    ),
    webUrl: getTag(event, "web") ?? null,
    owner,
    contributors: [...new Set([...getAllTags(event, "p"), ...setupUsers])],
    createdAt: event.created_at,
    status: getTag(event, "status") ?? "active",
    defaultBranch: getTag(event, "default-branch") ?? "main",
    repoAddress: `${KIND_REPO_ANNOUNCEMENT}:${owner}:${dtag}`,
  };
}

function eventToExplicitProject(
  event: RelayEvent,
  repositoriesByAddress: ReadonlyMap<string, Repository>,
): Project | null {
  const dtag = getTag(event, "d");
  const name = getTag(event, "name");
  if (
    event.kind !== KIND_PROJECT_ANNOUNCEMENT ||
    !dtag ||
    !name ||
    !isValidIdentifier(dtag) ||
    !isValidPubkey(event.pubkey)
  ) {
    return null;
  }

  const membershipTags = event.tags.filter((tag) => tag[0] === "a");
  const repositoryAddresses: string[] = [];
  const seen = new Set<string>();
  let primaryRepositoryAddress: string | null = null;
  for (const membershipTag of membershipTags) {
    const repositoryAddress = membershipTag[1];
    if (
      !repositoryAddress ||
      !parseRepositoryAddress(repositoryAddress) ||
      seen.has(repositoryAddress)
    ) {
      return null;
    }
    seen.add(repositoryAddress);
    repositoryAddresses.push(repositoryAddress);
    if (membershipTag[3] === "primary") {
      if (primaryRepositoryAddress) return null;
      primaryRepositoryAddress = repositoryAddress;
    }
  }

  if (
    (repositoryAddresses.length > 0 && !primaryRepositoryAddress) ||
    (repositoryAddresses.length === 0 && primaryRepositoryAddress)
  ) {
    return null;
  }

  const owner = event.pubkey.toLowerCase();
  return {
    id: `${owner}:${dtag}`,
    dtag,
    name,
    description: event.content ?? "",
    owner,
    createdAt: event.created_at,
    projectChannelId: getTag(event, "h") ?? null,
    status: getTag(event, "status") ?? "active",
    projectAddress: `${KIND_PROJECT_ANNOUNCEMENT}:${owner}:${dtag}`,
    primaryRepositoryAddress,
    repositoryAddresses,
    repositories: repositoryAddresses.flatMap((address) => {
      const repository = repositoriesByAddress.get(address);
      return repository ? [repository] : [];
    }),
    legacy: false,
  };
}

function repositoryToLegacyProject(repository: Repository): Project {
  return {
    id: repository.id,
    dtag: repository.dtag,
    name: repository.name,
    description: repository.description,
    owner: repository.owner,
    createdAt: repository.createdAt,
    projectChannelId: null,
    status: repository.status,
    projectAddress: repository.repoAddress,
    primaryRepositoryAddress: repository.repoAddress,
    repositoryAddresses: [repository.repoAddress],
    repositories: [repository],
    legacy: true,
  };
}

export function buildProjectReadModels({
  projectEvents,
  repositoryEvents,
  relayOrigin,
}: BuildProjectReadModelsInput): Project[] {
  const repositories = deduplicateAddressableEvents(repositoryEvents).flatMap(
    (event) => {
      const repository = eventToRepository(event, relayOrigin);
      return repository ? [repository] : [];
    },
  );
  const repositoriesByAddress = new Map(
    repositories.map((repository) => [repository.repoAddress, repository]),
  );

  const explicitProjects = deduplicateAddressableEvents(projectEvents).flatMap(
    (event) => {
      const project = eventToExplicitProject(event, repositoriesByAddress);
      return project ? [project] : [];
    },
  );
  const referencedRepositories = new Set(
    explicitProjects.flatMap((project) => project.repositoryAddresses),
  );
  const legacyProjects = repositories
    .filter((repository) => !referencedRepositories.has(repository.repoAddress))
    .map(repositoryToLegacyProject);

  return [...explicitProjects, ...legacyProjects].sort(
    (left, right) => right.createdAt - left.createdAt,
  );
}
