import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  fetchRepositoryActivitySummaries,
  type Project,
} from "@/features/projects/hooks";

/** Fetches repository-specific activity for the repositories in these projects. */
export function useRepositoryActivitySummariesQuery(projects: Project[]) {
  const repositories = React.useMemo(
    () => [
      ...new Map(
        projects
          .flatMap((project) => project.repositories)
          .map((repository) => [repository.repoAddress, repository]),
      ).values(),
    ],
    [projects],
  );
  const repoAddresses = React.useMemo(
    () => repositories.map((repository) => repository.repoAddress).sort(),
    [repositories],
  );

  return useQuery({
    enabled: repoAddresses.length > 0,
    queryKey: ["projects", "activity-summaries", "repositories", repoAddresses],
    queryFn: () => fetchRepositoryActivitySummaries(repositories),
    staleTime: 30_000,
  });
}
