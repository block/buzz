import type { Project, Repository } from "../projectModels";
import type { ProjectIssue } from "../projectIssues.mjs";
import { listProjectBoundChannels } from "./projectRelatedChannels";

export type ChannelNextUpItem = {
  project: Project;
  repository: Repository;
  issue: ProjectIssue;
};

/** Resolve existing project bindings without treating a task's origin as access authority. */
export function channelNextUpProjects(
  projects: readonly Project[],
  channelId: string,
): Project[] {
  return projects.filter(
    (project) =>
      project.repositories.length > 0 &&
      listProjectBoundChannels(project).some(
        (channel) => channel.channelId === channelId,
      ),
  );
}

/** Attach tasks to an accessible repository in this channel's existing projects. */
export function channelNextUpItems(
  projects: readonly Project[],
  issues: readonly ProjectIssue[],
  channelId: string,
): ChannelNextUpItem[] {
  const repositories = new Map<
    string,
    { project: Project; repository: Repository }
  >();
  for (const project of channelNextUpProjects(projects, channelId)) {
    for (const repository of project.repositories) {
      if (!repositories.has(repository.repoAddress)) {
        repositories.set(repository.repoAddress, { project, repository });
      }
    }
  }
  const seen = new Set<string>();
  return issues.flatMap((issue) => {
    const context = repositories.get(issue.repoAddress ?? "");
    const key = `${issue.repoAddress ?? ""}:${issue.id}`;
    if (issue.channelId !== channelId || !context || seen.has(key)) return [];
    seen.add(key);
    return [{ ...context, issue }];
  });
}

/** Select active tasks; unknown status must never hide a potentially open task. */
export function filterChannelNextUpItems(
  items: readonly ChannelNextUpItem[],
  assignee: string | null,
  statusesComplete = true,
): ChannelNextUpItem[] {
  const key = assignee?.toLowerCase();
  return items.filter(
    ({ issue }) =>
      (!statusesComplete ||
        (issue.status !== "Done" && issue.status !== "Closed")) &&
      (!key || issue.assignees.some((pubkey) => pubkey.toLowerCase() === key)),
  );
}
