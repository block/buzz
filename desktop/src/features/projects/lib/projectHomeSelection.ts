import type { Project } from "@/features/projects/projectModels";

/** Resolves the canonical visible project home for a channel. */
export function findProjectHomeByChannelId(
  channelId: string | null | undefined,
  projects: readonly Project[],
): Project | null {
  if (!channelId) return null;
  const matching = projects
    .filter(
      (project) =>
        !project.legacy &&
        project.projectChannelId === channelId &&
        hasAuthoritativeHomeBinding(project),
    )
    .sort((left, right) => left.createdAt - right.createdAt);
  return (
    matching.find((project) => project.visibility !== "unlisted") ??
    matching[0] ??
    null
  );
}

export type ProjectHomeCandidate = {
  owner: string;
  projectChannelId: string | null;
  repositories: ReadonlyArray<{
    channelId?: string | null;
    maintainers?: ReadonlyArray<string>;
    owner: string;
  }>;
};

export function hasAuthoritativeHomeBinding(
  project: ProjectHomeCandidate,
): boolean {
  const channelId = project.projectChannelId;
  if (!channelId) return false;

  const projectOwner = project.owner.toLowerCase();
  return project.repositories.some((repository) => {
    if (repository.channelId !== channelId) return false;
    if (repository.owner.toLowerCase() === projectOwner) return true;
    return repository.maintainers?.some(
      (maintainer) => maintainer.toLowerCase() === projectOwner,
    );
  });
}

export function isProjectHomeChannel(
  channelId: string | null | undefined,
  projects: ReadonlyArray<ProjectHomeCandidate>,
): boolean {
  if (!channelId) return false;
  return projects.some(
    (project) =>
      project.projectChannelId === channelId &&
      hasAuthoritativeHomeBinding(project),
  );
}
