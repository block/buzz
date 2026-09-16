/** Bound project and repository channels shown on the Projects overview. */

export type ProjectRelatedChannelSource = {
  id: string;
  name: string;
  projectChannelId: string | null;
  relatedChannelIds?: readonly string[];
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

/** One display row per distinct channel within a project. */
export type ProjectRelatedChannelDisplayRow = {
  channelId: string;
  projectId: string;
  projectName: string;
  repositoryNames: string[];
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
      repositoryChannelIds.add(projectChannelId);
    }
    for (const relatedChannelId of project.relatedChannelIds ?? []) {
      const channelId = trimmedChannelId(relatedChannelId);
      if (!channelId || repositoryChannelIds.has(channelId)) continue;
      repositoryChannelIds.add(channelId);
      rows.push({
        channelId,
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

/** Collapses repository bindings that point at the same project channel. */
export function collapseProjectRelatedChannelRows(
  rows: readonly ProjectRelatedChannelRow[],
): ProjectRelatedChannelDisplayRow[] {
  const collapsed = new Map<string, ProjectRelatedChannelDisplayRow>();
  for (const row of rows) {
    const key = `${row.projectId}:${row.channelId}`;
    const current = collapsed.get(key);
    if (current) {
      if (
        row.repositoryName &&
        !current.repositoryNames.includes(row.repositoryName)
      ) {
        current.repositoryNames.push(row.repositoryName);
      }
      continue;
    }
    collapsed.set(key, {
      channelId: row.channelId,
      projectId: row.projectId,
      projectName: row.projectName,
      repositoryNames: row.repositoryName ? [row.repositoryName] : [],
    });
  }
  return [...collapsed.values()];
}

/** Stable key for one collapsed project-channel row. */
export function projectRelatedChannelDisplayRowKey(
  row: ProjectRelatedChannelDisplayRow,
) {
  return `${row.channelId}:${row.projectId}`;
}

export type ProjectBoundChannel = {
  channelId: string;
  repositoryId: string | null;
  role: "home" | "related";
};

/**
 * Unique channels bound to one project: the home stream first, then each
 * repository channel that is not already the home channel.
 */
export function listProjectBoundChannels(
  project: Pick<
    ProjectRelatedChannelSource,
    "projectChannelId" | "relatedChannelIds" | "repositories"
  >,
): ProjectBoundChannel[] {
  const channels: ProjectBoundChannel[] = [];
  const seen = new Set<string>();
  const homeChannelId = trimmedChannelId(project.projectChannelId);
  if (homeChannelId) {
    channels.push({
      channelId: homeChannelId,
      repositoryId: null,
      role: "home",
    });
    seen.add(homeChannelId);
  }
  for (const relatedChannelId of project.relatedChannelIds ?? []) {
    const channelId = trimmedChannelId(relatedChannelId);
    if (!channelId || seen.has(channelId)) continue;
    channels.push({
      channelId,
      repositoryId: null,
      role: "related",
    });
    seen.add(channelId);
  }
  for (const repository of project.repositories) {
    const channelId = trimmedChannelId(repository.channelId);
    if (!channelId || seen.has(channelId)) continue;
    channels.push({
      channelId,
      repositoryId: repository.id,
      role: "related",
    });
    seen.add(channelId);
  }
  return channels;
}

/**
 * Nested sidebar rows under a project: bound streams except the home
 * channel, which is the project row itself.
 */
export function listProjectChildChannels(
  project: Pick<
    ProjectRelatedChannelSource,
    "projectChannelId" | "relatedChannelIds" | "repositories"
  >,
): ProjectBoundChannel[] {
  return listProjectBoundChannels(project).filter(
    (channel) => channel.role !== "home",
  );
}
