import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  buildProjectsOverviewAgentContext,
  type ProjectDetailAgentContext,
} from "@/features/projects/lib/projectDetailAgentContext";
import { projectsSectionTitle } from "./projectsSectionMeta";
import {
  buildProjectsViewAgentContextItems,
  type ProjectsViewAgentContextInput,
} from "./buildProjectsViewAgentContext";

const EMPTY_ISSUES: ProjectsViewAgentContextInput["issues"] = [];
const EMPTY_PULL_REQUESTS: ProjectsViewAgentContextInput["pullRequests"] = [];

export function useProjectsOverviewAgentContext(
  input: Omit<
    ProjectsViewAgentContextInput,
    "channels" | "issues" | "pullRequests"
  > &
    Partial<Pick<ProjectsViewAgentContextInput, "issues" | "pullRequests">>,
) {
  const {
    filter,
    issues = EMPTY_ISSUES,
    projects,
    pullRequests = EMPTY_PULL_REQUESTS,
    snapshots,
    visibleIssues,
    visibleProjects,
    visiblePullRequests,
    visibleRepositories,
  } = input;
  const [agentContext, setAgentContext] =
    React.useState<ProjectDetailAgentContext | null>(null);
  const projectChannelsQuery = useChannelsQuery({
    enabled: filter === "channels",
  });
  const overviewContext = React.useMemo(
    () =>
      buildProjectsOverviewAgentContext(
        projectsSectionTitle(filter),
        buildProjectsViewAgentContextItems({
          channels: projectChannelsQuery.data ?? [],
          filter,
          issues,
          projects,
          pullRequests,
          snapshots,
          visibleIssues,
          visibleProjects,
          visiblePullRequests,
          visibleRepositories,
        }),
      ),
    [
      filter,
      issues,
      projectChannelsQuery.data,
      projects,
      pullRequests,
      snapshots,
      visibleIssues,
      visibleProjects,
      visiblePullRequests,
      visibleRepositories,
    ],
  );

  React.useEffect(() => {
    setAgentContext((context) =>
      context?.repoAddress === "projects:overview" ? overviewContext : context,
    );
  }, [overviewContext]);

  return { agentContext, overviewContext, setAgentContext };
}
