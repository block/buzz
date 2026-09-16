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
  /**
   * Extra streams linked to this project via repeatable
   * `buzz-related-channel` tags. Client convention: NIP-MP treats the tag as
   * unrecognized metadata, so older readers ignore it.
   */
  relatedChannelIds: string[];
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

/** True for an announced NIP-MP project, excluding repository-only read models. */
export function isExplicitProject(project: Project): boolean {
  return !project.legacy;
}

type BuildProjectReadModelsInput = {
  projectEvents: RelayEvent[];
  repositoryEvents: RelayEvent[];
  /** NIP-09 kind:5 deletion events relevant to projects and repositories. */
  deletionEvents?: RelayEvent[];
  relayOrigin?: string | null;
  hiddenAddresses?: ReadonlySet<string>;
  /**
   * When set, the viewer's own unlisted projects stay in the collection so the
   * creator can still open them. Other viewers keep the NIP-MP fold: unlisted
   * projects are absent and do not claim members.
   */
  viewerPubkey?: string | null;
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

/**
 * Validates a pubkey as a lowercase-only 64-hex string, per NIP-MP rule
 * `member-coordinate-malformed`: owner hex MUST be lowercase so that `#a`
 * filter matching (which is byte-exact) can resolve the coordinate.
 */
function isValidProjectMemberOwner(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

/** NIP-MP rule `member-cap`: a project may carry at most 64 member `a` tags. */
export const MAX_PROJECT_MEMBERS = 64;

export function isValidProjectChannelId(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    value,
  );
}

/** Repeatable project tag naming an extra stream besides `buzz-channel`. */
export const PROJECT_RELATED_CHANNEL_TAG = "buzz-related-channel";

/** Cap extra project streams so a tag list cannot grow without bound. */
export const MAX_PROJECT_RELATED_CHANNELS = 64;

const SINGLETON_METADATA_TAGS = [
  "name",
  "description",
  "buzz-channel",
  "buzz-visibility",
] as const;

const MAX_METADATA_TAG_BYTES: Record<string, number> = {
  name: 256,
  description: 2_048,
  "buzz-channel": 256,
  "buzz-visibility": 256,
};

/**
 * Validates the NIP-MP tag/content envelope for a kind:30621 project event.
 * Shared by both the read parser (`eventToExplicitProject`) and the write
 * helper (`buildProjectPatchTemplate`) so Desktop's input and output agree on
 * which heads are valid.
 *
 * Throws a descriptive error on the first violation found.
 */
export function validateProjectEventEnvelope(
  tags: string[][],
  content: string,
): void {
  // NIP-MP rule `d-cardinality`: exactly one `d` tag is required.
  const dTags = tags.filter((tag) => tag[0] === "d");
  if (dTags.length !== 1 || !dTags[0][1]) {
    throw new Error(
      `NIP-MP: expected exactly one non-empty 'd' tag, found ${dTags.length}.`,
    );
  }
  const dtag = dTags[0][1];
  if (!isValidDTag(dtag)) {
    throw new Error(`NIP-MP: 'd' tag value exceeds the maximum byte length.`);
  }

  // NIP-MP rule `metadata-cardinality`: at most one each of the singleton tags.
  const encoder = new TextEncoder();
  for (const tagName of SINGLETON_METADATA_TAGS) {
    const count = tags.filter((tag) => tag[0] === tagName).length;
    if (count > 1) {
      throw new Error(
        `NIP-MP: duplicate '${tagName}' tag — at most one is permitted.`,
      );
    }
  }

  // NIP-MP rule `metadata-length`: per-field byte caps.
  for (const [tagName, maxBytes] of Object.entries(MAX_METADATA_TAG_BYTES)) {
    const value = tags.find((tag) => tag[0] === tagName)?.[1];
    if (value !== undefined && encoder.encode(value).byteLength > maxBytes) {
      throw new Error(
        `NIP-MP: '${tagName}' tag value exceeds the ${maxBytes}-byte limit.`,
      );
    }
  }

  // NIP-MP rule `member-cap`: at most 64 `a` membership tags.
  const memberTags = tags.filter((tag) => tag[0] === "a");
  if (memberTags.length > MAX_PROJECT_MEMBERS) {
    throw new Error(
      `NIP-MP: project exceeds the ${MAX_PROJECT_MEMBERS}-member limit.`,
    );
  }

  // NIP-MP rule `member-coordinate-malformed` + `member-arity`:
  // each `a` tag must be a valid repository coordinate with a lowercase owner,
  // and must carry 2 or 3 elements (address + optional relay hint).
  const seenAddresses = new Set<string>();
  for (const tag of memberTags) {
    const address = tag[1];
    if (!address) {
      throw new Error("NIP-MP: 'a' tag is missing a repository address.");
    }
    if (tag.length !== 2 && tag.length !== 3) {
      throw new Error(
        `NIP-MP: 'a' tag for '${address}' must have 2 or 3 elements.`,
      );
    }
    const parsed = parseRepositoryAddress(address);
    if (!parsed) {
      throw new Error(
        `NIP-MP: invalid repository address '${address}' — expected '30617:<lowercase-hex64>:<dtag>'.`,
      );
    }
    if (seenAddresses.has(address)) {
      throw new Error(`NIP-MP: duplicate repository address '${address}'.`);
    }
    seenAddresses.add(address);
  }

  void content; // content is preserved verbatim; no constraint in NIP-MP.
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
  return isValidProjectMemberOwner(owner) && isValidDTag(dtag)
    ? { owner, dtag }
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

export function eventToExplicitProject(
  event: RelayEvent,
  repositoriesByAddress: ReadonlyMap<string, Repository>,
  visibleRepositoriesByAddress: ReadonlyMap<string, Repository>,
): Project | null {
  if (
    event.kind !== KIND_PROJECT_ANNOUNCEMENT ||
    !isValidPubkey(event.pubkey)
  ) {
    return null;
  }

  // Delegate all NIP-MP envelope validation to the shared validator so the
  // read parser and the write helper (`buildProjectPatchTemplate`) agree on
  // which heads are valid. The parser rejects invalid events silently (returns
  // null) while the write helper throws, so wrap in a try/catch here.
  try {
    validateProjectEventEnvelope(event.tags, event.content);
  } catch {
    return null;
  }

  // After validation we know: exactly one `d` tag with a valid value, at most
  // 64 `a` tags with valid repo coordinates, no duplicate addresses, and all
  // singleton metadata tags within their byte caps.
  const dtag = event.tags.find((tag) => tag[0] === "d")?.[1] ?? "";
  const membershipTags = event.tags.filter((tag) => tag[0] === "a");
  const repositoryAddresses: string[] = [];
  const repositoryRelayHints: Record<string, string> = {};
  for (const membershipTag of membershipTags) {
    const repositoryAddress = membershipTag[1];
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
  const projectChannelId =
    channel && isValidProjectChannelId(channel) ? channel : null;
  const relatedChannelIds = [
    ...new Set(
      getAllTags(event, PROJECT_RELATED_CHANNEL_TAG).filter(
        (channelId) =>
          isValidProjectChannelId(channelId) && channelId !== projectChannelId,
      ),
    ),
  ].slice(0, MAX_PROJECT_RELATED_CHANNELS);
  return {
    id: projectAddress,
    dtag,
    name: getTag(event, "name") ?? dtag,
    description: getTag(event, "description") ?? "",
    owner,
    createdAt: event.created_at,
    projectChannelId,
    relatedChannelIds,
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
    relatedChannelIds: [],
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

/**
 * Builds deletion thresholds from relay-accepted tombstones. The relay has
 * already enforced that each signer controls the addressed coordinate,
 * including Buzz's NIP-OA owner delegation for agent-authored events.
 */
function buildDeletionThresholds(
  deletionEvents: RelayEvent[],
): Map<string, number> {
  const thresholds = new Map<string, number>();
  for (const event of deletionEvents) {
    for (const tag of event.tags) {
      if (tag[0] !== "a" || !tag[1]) continue;
      const coordinate = tag[1];
      const existing = thresholds.get(coordinate);
      if (existing === undefined || event.created_at > existing) {
        thresholds.set(coordinate, event.created_at);
      }
    }
  }
  return thresholds;
}

function projectIsListingEligible(
  project: Project,
  viewerPubkey: string | null | undefined,
): boolean {
  if (project.visibility !== "unlisted") return true;
  return Boolean(
    viewerPubkey && project.owner === viewerPubkey.trim().toLowerCase(),
  );
}

export function buildProjectReadModels({
  projectEvents,
  repositoryEvents,
  deletionEvents = [],
  relayOrigin,
  hiddenAddresses = new Set(),
  viewerPubkey,
}: BuildProjectReadModelsInput): Project[] {
  const deletionThresholds = buildDeletionThresholds(deletionEvents);

  /** Returns true when the event's addressable coordinate has been deleted. */
  function isDeleted(event: RelayEvent): boolean {
    const dtag = event.tags.find((tag) => tag[0] === "d")?.[1];
    if (!dtag) return false;
    const coordinate = `${event.kind}:${event.pubkey.toLowerCase()}:${dtag}`;
    const threshold = deletionThresholds.get(coordinate);
    return threshold !== undefined && event.created_at <= threshold;
  }

  const repositories = deduplicateAddressableEvents(repositoryEvents)
    .filter((event) => !isDeleted(event))
    .flatMap((event) => {
      const repository = eventToRepository(event, relayOrigin);
      return repository ? [repository] : [];
    });
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

  const explicitProjects = deduplicateAddressableEvents(projectEvents)
    .filter((event) => !isDeleted(event))
    .flatMap((event) => {
      const project = eventToExplicitProject(
        event,
        repositoriesByAddress,
        visibleRepositoriesByAddress,
      );
      return project &&
        projectIsListingEligible(project, viewerPubkey) &&
        !hiddenAddresses.has(project.projectAddress)
        ? [project]
        : [];
    });
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

/** Returns the optimistic read model after linking an extra project stream. */
export function addRelatedChannelToProject(
  project: Project,
  channelId: string,
  createdAt: number,
): Project {
  const relatedChannelIds = [
    ...new Set([...(project.relatedChannelIds ?? []), channelId]),
  ].filter(
    (id) => id !== project.projectChannelId && isValidProjectChannelId(id),
  );
  return {
    ...project,
    createdAt,
    relatedChannelIds: relatedChannelIds.slice(0, MAX_PROJECT_RELATED_CHANNELS),
  };
}
