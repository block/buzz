/** Bound project and repository channels shown on the Projects overview. */

export type ProjectRelatedChannelSource = {
  id: string;
  name: string;
  projectChannelId: string | null;
  repositories: Array<{
    id: string;
    name: string;
    channelId?: string | null;
  }>;
};

export type ProjectRelatedChannelRow = {
  channelId: string;
  projectId: string;
  projectName: string;
  repositoryId: string | null;
  repositoryName: string | null;
};

function trimmedChannelId(value: string | null | undefined) {
  const channelId = value?.trim() ?? "";
  return channelId.length > 0 ? channelId : null;
}

/**
 * One row per project or repository binding so the overview list can show
 * which project and repository a channel belongs to. When a project channel
 * is also bound to a repository in that project, only the repository row is
 * kept.
 */
export function collectProjectRelatedChannelRows(
  projects: readonly ProjectRelatedChannelSource[],
): ProjectRelatedChannelRow[] {
  const rows: ProjectRelatedChannelRow[] = [];
  for (const project of projects) {
    const repositoryChannelIds = new Set<string>();
    for (const repository of project.repositories) {
      const channelId = trimmedChannelId(repository.channelId);
      if (!channelId) continue;
      repositoryChannelIds.add(channelId);
      rows.push({
        channelId,
        projectId: project.id,
        projectName: project.name,
        repositoryId: repository.id,
        repositoryName: repository.name,
      });
    }
    const projectChannelId = trimmedChannelId(project.projectChannelId);
    if (projectChannelId && !repositoryChannelIds.has(projectChannelId)) {
      rows.push({
        channelId: projectChannelId,
        projectId: project.id,
        projectName: project.name,
        repositoryId: null,
        repositoryName: null,
      });
    }
  }
  return rows;
}

export function uniqueProjectRelatedChannelCount(
  projects: readonly ProjectRelatedChannelSource[],
) {
  return new Set(
    collectProjectRelatedChannelRows(projects).map((row) => row.channelId),
  ).size;
}

export function projectRelatedChannelRowKey(row: ProjectRelatedChannelRow) {
  return `${row.channelId}:${row.projectId}:${row.repositoryId ?? "project"}`;
}
