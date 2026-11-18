import type * as React from "react";

import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { ProjectPanelState } from "./ProjectPanelState";

/** Which center pane `PullRequestsPanel` should show. A retained selected
 * review wins over an empty list so refetching does not flash "No reviews yet." */
export function pullRequestsPanelKind({
  isLoading,
  pullRequests,
  selectedPullRequest,
}: {
  isLoading: boolean;
  pullRequests: { length: number };
  selectedPullRequest: unknown;
}): "loading" | "detail" | "empty" | "list" {
  if (isLoading && !selectedPullRequest) return "loading";
  if (selectedPullRequest) return "detail";
  if (pullRequests.length === 0) return "empty";
  return "list";
}

/** Loading / empty / detail / list switch used by `PullRequestsPanel`. */
export function PullRequestsPanelSurface({
  detail,
  error,
  isLoading,
  list,
  pullRequests,
  selectedPullRequest,
}: {
  detail: React.ReactNode;
  error: unknown;
  isLoading: boolean;
  list: React.ReactNode;
  pullRequests: { length: number };
  selectedPullRequest: unknown;
}) {
  const kind = pullRequestsPanelKind({
    isLoading,
    pullRequests,
    selectedPullRequest,
  });
  if (kind === "loading") {
    return <BuzzLoadingState label="Loading reviews" />;
  }
  if (kind === "detail") {
    return detail;
  }
  if (kind === "empty") {
    return (
      <ProjectPanelState
        description={
          error
            ? "Refresh the repository and try again."
            : "Reviews opened for this repository will appear here."
        }
        error={Boolean(error)}
        testId="project-pull-requests-empty"
        title={error ? "Could not load reviews" : "No reviews yet"}
      />
    );
  }
  return list;
}
