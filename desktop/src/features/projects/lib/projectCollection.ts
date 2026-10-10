import type { Project, Repository } from "@/features/projects/projectModels";

function withAbsorbedRepository(
  project: Project,
  repository: Repository,
): Project {
  if (project.repositoryAddresses.includes(repository.repoAddress)) {
    return project;
  }
  return {
    ...project,
    primaryRepositoryAddress:
      project.primaryRepositoryAddress ?? repository.repoAddress,
    repositories: [...project.repositories, repository],
    repositoryAddresses: [
      ...project.repositoryAddresses,
      repository.repoAddress,
    ],
  };
}

function repositoryAuthorizesProjectOwner(
  project: Project,
  repository: Repository,
): boolean {
  const projectOwner = project.owner.toLowerCase();
  if (repository.owner.toLowerCase() === projectOwner) return true;
  return Boolean(
    repository.maintainers?.some(
      (maintainer) => maintainer.toLowerCase() === projectOwner,
    ),
  );
}

/**
 * The owner already published a project event that lists other repositories
 * and omits this one. Desktop must not fold it back in: that is how an
 * auto-created slug repo survives `projects remove-repo`.
 */
function ownerOmittedRepository(
  project: Project,
  repository: Repository,
): boolean {
  return (
    repository.owner.toLowerCase() === project.owner.toLowerCase() &&
    project.repositoryAddresses.length > 0 &&
    !project.repositoryAddresses.includes(repository.repoAddress)
  );
}

function hostForStandaloneRepository(
  explicitProjects: Project[],
  repository: Repository,
): Project | undefined {
  const channelHost = repository.channelId
    ? explicitProjects.find(
        (project) =>
          project.projectChannelId === repository.channelId &&
          repositoryAuthorizesProjectOwner(project, repository) &&
          !ownerOmittedRepository(project, repository),
      )
    : undefined;
  if (channelHost) return channelHost;
  return explicitProjects.find(
    (project) =>
      project.owner === repository.owner &&
      project.dtag === repository.dtag &&
      !ownerOmittedRepository(project, repository),
  );
}

function repositoryBelongsOnProjectHome(
  project: Project,
  repository: Repository,
): boolean {
  return Boolean(
    (repository.channelId &&
      repository.channelId === project.projectChannelId &&
      repositoryAuthorizesProjectOwner(project, repository)) ||
      (repository.owner.toLowerCase() === project.owner.toLowerCase() &&
        repository.dtag === project.dtag),
  );
}

/**
 * Repositories already shown on the project (after absorb) that are not yet
 * on the signed `kind:30621` `a` tag set. The owner should bind them so
 * other clients see the same grouping.
 */
export function homeRepositoriesToBind(
  project: Project,
  signedAddresses: ReadonlyArray<string> | ReadonlySet<string>,
): Repository[] {
  const signed = new Set(signedAddresses);
  return project.repositories.filter((repository) => {
    if (signed.has(repository.repoAddress)) return false;
    if (!repositoryBelongsOnProjectHome(project, repository)) return false;
    // Absorbed view already includes the repo. Compare the signed a-tag set:
    // a non-empty set that omits the owner's own announcement is a removal,
    // not a repo the owner cannot bind themselves.
    if (
      repository.owner.toLowerCase() === project.owner.toLowerCase() &&
      signed.size > 0
    ) {
      return false;
    }
    return true;
  });
}

/**
 * After the NIP-MP fold, keep a repository off the standalone-project list
 * when it already belongs to a listing-eligible project's home channel, or
 * when the same owner already has an explicit project with that slug.
 *
 * Agents often announce a repo (`repos create --channel`) without
 * `projects add-repo`. Without this, the same work shows up as a second card.
 */
export function absorbStandaloneProjectRepositories(
  projects: Project[],
): Project[] {
  const explicitProjects = projects.filter((project) => !project.legacy);
  if (explicitProjects.length === 0) return projects;

  const absorbed = new Set<string>();
  let nextExplicit = explicitProjects;
  for (const card of projects) {
    if (!card.legacy) continue;
    const repository = card.repositories[0];
    if (!repository) continue;
    const host = hostForStandaloneRepository(nextExplicit, repository);
    if (!host) continue;
    absorbed.add(card.projectAddress);
    nextExplicit = nextExplicit.map((project) =>
      project.projectAddress === host.projectAddress
        ? withAbsorbedRepository(project, repository)
        : project,
    );
  }

  if (absorbed.size === 0) return projects;
  return [
    ...nextExplicit,
    ...projects.filter(
      (project) => project.legacy && !absorbed.has(project.projectAddress),
    ),
  ];
}
