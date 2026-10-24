import type { Project } from "@/features/projects/hooks";
import type {
  ProjectIssueListItem,
  ProjectPullRequestListItem,
  ProjectRepoSnapshot,
  Repository,
} from "@/features/projects/hooks";
import type { ProjectsOverviewAgentContextItem } from "@/features/projects/lib/projectDetailAgentContext";
import { collectProjectRelatedChannelRows } from "@/features/projects/lib/projectRelatedChannels";
import type { ProjectsFilter } from "@/features/projects/lib/projectsViewHelpers";
import type { Channel } from "@/shared/api/types";
import { buildProjectsActivityAgentContextItems } from "./ProjectsActivityFeed";

export type ProjectsViewAgentContextInput = {
  channels: Channel[];
  filter: ProjectsFilter;
  issues: ProjectIssueListItem[];
  projects: Project[];
  pullRequests: ProjectPullRequestListItem[];
  snapshots?: Record<string, ProjectRepoSnapshot>;
  visibleIssues: ProjectIssueListItem[];
  visibleProjects: Project[];
  visiblePullRequests: ProjectPullRequestListItem[];
  visibleRepositories: Array<{ project: Project; repository: Repository }>;
};

function detail(parts: Array<string | number | null | undefined>) {
  return parts
    .filter((part) => part !== null && part !== undefined && part !== "")
    .join(" · ");
}

export function buildProjectsViewAgentContextItems({
  channels,
  filter,
  issues,
  projects,
  pullRequests,
  snapshots,
  visibleIssues,
  visibleProjects,
  visiblePullRequests,
  visibleRepositories,
}: ProjectsViewAgentContextInput): ProjectsOverviewAgentContextItem[] {
  if (filter === "all") {
    return buildProjectsActivityAgentContextItems({
      issues,
      projects,
      pullRequests,
      snapshots,
    });
  }
  if (filter === "projects" || filter === "agents" || filter === "users") {
    return visibleProjects.map((project) => ({
      detail: detail([
        project.description,
        `${project.repositories.length} repositories`,
      ]),
      kind: "project",
      reference: project.id,
      title: project.name,
    }));
  }
  if (filter === "repositories") {
    return visibleRepositories.map(({ project, repository }) => ({
      detail: detail([repository.description, `Project: ${project.name}`]),
      kind: "repository",
      reference: repository.repoAddress,
      title: repository.name,
    }));
  }
  if (filter === "issues") {
    return visibleIssues.map(({ issue, project, repository }) => ({
      detail: detail([
        `Project: ${project.name}`,
        `Repository: ${repository.name}`,
        issue.status,
        issue.content,
      ]),
      kind: "task",
      reference: issue.id,
      title: issue.title,
    }));
  }
  if (filter === "prs") {
    return visiblePullRequests.map(({ project, pullRequest, repository }) => ({
      detail: detail([
        `Project: ${project.name}`,
        `Repository: ${repository.name}`,
        pullRequest.status,
        pullRequest.content,
      ]),
      kind: "review",
      reference: pullRequest.id,
      title: pullRequest.title,
    }));
  }

  const channelsById = new Map(
    channels.map((channel) => [channel.id, channel]),
  );
  return collectProjectRelatedChannelRows(projects).map((row) => {
    const channel = channelsById.get(row.channelId);
    return {
      detail: detail([
        `Project: ${row.projectName}`,
        row.repositoryName ? `Repository: ${row.repositoryName}` : null,
        channel?.description,
        channel ? `${channel.memberCount} members` : null,
      ]),
      kind: "channel",
      reference: row.channelId,
      title: `#${channel?.name ?? row.channelId.slice(0, 8)}`,
    };
  });
}
