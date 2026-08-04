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
  maintainers?: string[];
  channelId?: string | null;
  eventContent?: string;
  eventTags?: string[][];
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
  repositoryRelayHints?: Record<string, string>;
  repositories: Repository[];
  unavailableRepositoryAddresses?: string[];
  visibility?: "listed" | "unlisted";
  legacy: boolean;
};

type BuildProjectReadModelsInput = {
  projectEvents: RelayEvent[];
  repositoryEvents: RelayEvent[];
  relayOrigin?: string | null;
  hiddenAddresses?: ReadonlySet<string>;
};

const MAX_D_TAG_BYTES = 1_024;

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

function getAllTagValues(event: RelayEvent, name: string): string[] {
  return event.tags
    .filter((tag) => tag[0] === name)
    .flatMap((tag) => tag.slice(1))
    .filter((value) => value.length > 0);
}

function getCloneUrls(event: RelayEvent): string[] {
  const tag = event.tags.find((candidate) => candidate[0] === "clone");
  return tag?.slice(1).filter((value) => value.length > 0) ?? [];
}

function isValidDTag(value: string): boolean {
  return (
    value.length > 0 &&
    new TextEncoder().encode(value).byteLength <= MAX_D_TAG_BYTES
  );
}

function isValidPubkey(value: string): boolean {
  return /^[a-fA-F0-9]{64}$/.test(value);
}

export function isValidProjectChannelId(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    value,
  );
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
  return isValidPubkey(owner) && isValidDTag(dtag)
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
    !isValidDTag(dtag) ||
    !isValidPubkey(event.pubkey)
  ) {
    return null;
  }

  const owner = event.pubkey.toLowerCase();
  const setupUsers = getAllTags(event, "auth");
  const channel = getTag(event, "buzz-channel");
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
    channelId: channel && isValidProjectChannelId(channel) ? channel : null,
    eventContent: event.content,
    eventTags: event.tags.map((tag) => [...tag]),
    maintainers: getAllTagValues(event, "maintainers")
      .map((maintainer) => maintainer.toLowerCase())
      .filter(isValidPubkey),
  };
}

function eventToExplicitProject(
  event: RelayEvent,
  repositoriesByAddress: ReadonlyMap<string, Repository>,
  visibleRepositoriesByAddress: ReadonlyMap<string, Repository>,
): Project | null {
  const dtag = getTag(event, "d");
  if (
    event.kind !== KIND_PROJECT_ANNOUNCEMENT ||
    !dtag ||
    !isValidDTag(dtag) ||
    !isValidPubkey(event.pubkey)
  ) {
    return null;
  }

  const membershipTags = event.tags.filter((tag) => tag[0] === "a");
  const repositoryAddresses: string[] = [];
  const repositoryRelayHints: Record<string, string> = {};
  const seen = new Set<string>();
  for (const membershipTag of membershipTags) {
    const repositoryAddress = membershipTag[1];
    if (
      !repositoryAddress ||
      !parseRepositoryAddress(repositoryAddress) ||
      (membershipTag.length !== 2 && membershipTag.length !== 3) ||
      seen.has(repositoryAddress)
    ) {
      return null;
    }
    seen.add(repositoryAddress);
    repositoryAddresses.push(repositoryAddress);
    if (membershipTag[2]) {
      repositoryRelayHints[repositoryAddress] = membershipTag[2];
    }
  }
  repositoryAddresses.sort();
  const primaryRepositoryAddress =
    repositoryAddresses.find(
      (address) => visibleRepositoriesByAddress.get(address)?.dtag === dtag,
    ) ??
    repositoryAddresses.find((address) =>
      visibleRepositoriesByAddress.has(address),
    ) ??
    null;

  const owner = event.pubkey.toLowerCase();
  const projectAddress = `${KIND_PROJECT_ANNOUNCEMENT}:${owner}:${dtag}`;
  const rawVisibility = getTag(event, "buzz-visibility");
  const visibility =
    rawVisibility === "unlisted" ? ("unlisted" as const) : ("listed" as const);
  const channel = getTag(event, "buzz-channel");
  return {
    id: projectAddress,
    dtag,
    name: getTag(event, "name") ?? dtag,
    description: getTag(event, "description") ?? "",
    owner,
    createdAt: event.created_at,
    projectChannelId:
      channel && isValidProjectChannelId(channel) ? channel : null,
    status: visibility === "listed" ? "active" : "unlisted",
    projectAddress,
    primaryRepositoryAddress,
    repositoryAddresses,
    repositoryRelayHints,
    repositories: repositoryAddresses.flatMap((address) => {
      const repository = visibleRepositoriesByAddress.get(address);
      return repository ? [repository] : [];
    }),
    unavailableRepositoryAddresses: repositoryAddresses.filter(
      (address) => !repositoriesByAddress.has(address),
    ),
    visibility,
    legacy: false,
  };
}

function repositoryToLegacyProject(repository: Repository): Project {
  return {
    id: repository.repoAddress,
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
    repositoryRelayHints: {},
    repositories: [repository],
    unavailableRepositoryAddresses: [],
    visibility: "listed",
    legacy: true,
  };
}

export function buildProjectReadModels({
  projectEvents,
  repositoryEvents,
  relayOrigin,
  hiddenAddresses = new Set(),
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
  const visibleRepositories = repositories.filter(
    (repository) => !hiddenAddresses.has(repository.repoAddress),
  );
  const visibleRepositoriesByAddress = new Map(
    visibleRepositories.map((repository) => [
      repository.repoAddress,
      repository,
    ]),
  );

  const explicitProjects = deduplicateAddressableEvents(projectEvents).flatMap(
    (event) => {
      const project = eventToExplicitProject(
        event,
        repositoriesByAddress,
        visibleRepositoriesByAddress,
      );
      return project &&
        project.visibility === "listed" &&
        !hiddenAddresses.has(project.projectAddress)
        ? [project]
        : [];
    },
  );
  const claimedRepositories = new Set(
    explicitProjects.flatMap((project) =>
      project.repositoryAddresses.filter((address) => {
        const repository = repositoriesByAddress.get(address);
        return (
          repository &&
          (repository.owner === project.owner ||
            repository.maintainers?.includes(project.owner))
        );
      }),
    ),
  );
  const legacyProjects = visibleRepositories
    .filter((repository) => !claimedRepositories.has(repository.repoAddress))
    .map(repositoryToLegacyProject);

  return [...explicitProjects, ...legacyProjects].sort(
    (left, right) => right.createdAt - left.createdAt,
  );
}

export function selectProjectRepository(
  project: Project | null | undefined,
  requestedRepositoryId: string | null | undefined,
): Repository | null {
  if (!project) return null;

  const requested = requestedRepositoryId
    ? project.repositories.find(
        (repository) => repository.id === requestedRepositoryId,
      )
    : null;
  if (requested) return requested;

  return (
    project.repositories.find(
      (repository) =>
        repository.repoAddress === project.primaryRepositoryAddress,
    ) ??
    project.repositories[0] ??
    null
  );
}

/** Returns the optimistic read model after adding a resolved repository. */
export function addRepositoryToProject(
  project: Project,
  repository: Repository,
  createdAt: number,
): Project {
  const projectAddress = `${KIND_PROJECT_ANNOUNCEMENT}:${project.owner}:${project.dtag}`;
  const repositoryAddresses = [
    ...new Set([...project.repositoryAddresses, repository.repoAddress]),
  ].sort();
  const repositories = [
    ...project.repositories.filter(
      (candidate) => candidate.repoAddress !== repository.repoAddress,
    ),
    repository,
  ].sort((left, right) => left.repoAddress.localeCompare(right.repoAddress));

  return {
    ...project,
    id: projectAddress,
    createdAt,
    legacy: false,
    projectAddress,
    primaryRepositoryAddress:
      repositories.find((candidate) => candidate.dtag === project.dtag)
        ?.repoAddress ??
      repositories[0]?.repoAddress ??
      null,
    repositoryAddresses,
    repositories,
    unavailableRepositoryAddresses:
      project.unavailableRepositoryAddresses?.filter(
        (address) => address !== repository.repoAddress,
      ) ?? [],
  };
}
