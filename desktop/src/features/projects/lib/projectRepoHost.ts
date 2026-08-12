import { effectiveCloneUrls } from "./projectCloneUrl";

export type ProjectRepoHost =
  | { kind: "buzz" }
  | { kind: "external"; host: string }
  | { kind: "unresolved" };

/**
 * Classifies the canonical git remote using the same origin and path boundary
 * enforced by the Tauri git commands. This is presentation/query gating only;
 * Rust remains the security boundary for clone operations.
 */
export function projectRepoHost(
  cloneUrl: string | null | undefined,
  relayOrigin: string | null | undefined,
): ProjectRepoHost {
  if (!cloneUrl || !relayOrigin) return { kind: "unresolved" };

  try {
    const clone = new URL(cloneUrl);
    const relay = new URL(relayOrigin);
    const isBuzzPath = /^\/git\/[0-9a-f]{64}\/[^/]+\/?$/i.test(clone.pathname);

    if (clone.origin === relay.origin && isBuzzPath) {
      return { kind: "buzz" };
    }

    return { kind: "external", host: clone.host };
  } catch {
    return { kind: "unresolved" };
  }
}

type RepositoryHostInput = {
  cloneUrls: string[];
  dtag: string;
  owner: string;
  repoAddress?: string;
};

export function projectRepoHostForRepository(
  repository: RepositoryHostInput | null | undefined,
  relayOrigin: string | null | undefined,
): ProjectRepoHost {
  if (!repository) return { kind: "unresolved" };
  const cloneUrl = effectiveCloneUrls(
    repository.cloneUrls,
    relayOrigin,
    repository.owner,
    repository.dtag,
  )[0];
  return projectRepoHost(cloneUrl, relayOrigin);
}

/**
 * Human-readable location of a repository's git data — "github.com/block/buzz"
 * for external repos (host + path, `.git` stripped), or "owner/repo" for
 * Buzz-hosted ones (the relay host and full owner pubkey carry no signal;
 * `ownerLabel` should be the resolved profile name, falling back to a
 * shortened pubkey). Returns `null` when no clone URL can be resolved.
 */
export function repositoryDisplayPath(
  repository: RepositoryHostInput | null | undefined,
  relayOrigin: string | null | undefined,
  ownerLabel?: string | null,
): string | null {
  if (!repository) return null;
  const cloneUrl = effectiveCloneUrls(
    repository.cloneUrls,
    relayOrigin,
    repository.owner,
    repository.dtag,
  )[0];
  if (!cloneUrl) return null;

  if (projectRepoHost(cloneUrl, relayOrigin).kind === "buzz") {
    const owner = ownerLabel?.trim() || `${repository.owner.slice(0, 8)}…`;
    return `${owner}/${repository.dtag}`;
  }

  try {
    const url = new URL(cloneUrl);
    const path = url.pathname.replace(/\.git$/, "").replace(/\/+$/, "");
    return `${url.host}${path}`;
  } catch {
    return null;
  }
}

export function projectRepoHostForProject(
  project:
    | RepositoryHostInput
    | {
        primaryRepositoryAddress: string | null;
        repositories: RepositoryHostInput[];
      }
    | null
    | undefined,
  relayOrigin: string | null | undefined,
): ProjectRepoHost {
  if (!project) return { kind: "unresolved" };
  if (!("repositories" in project)) {
    return projectRepoHostForRepository(project, relayOrigin);
  }

  const repository =
    project.repositories.find(
      (candidate) => candidate.repoAddress === project.primaryRepositoryAddress,
    ) ??
    project.repositories[0] ??
    null;
  return projectRepoHostForRepository(repository, relayOrigin);
}
