import type { Project, ProjectRepoSnapshot } from "@/features/projects/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Tooltip for the push/pull sync buttons, e.g. "Pull 2 remote commits". */
export function pushPullTitle(
  verb: "Push" | "Pull",
  count: number | undefined,
  side: "local" | "remote",
) {
  if (!count) return `${verb} ${side} commits`;
  return `${verb} ${count} ${side} ${count === 1 ? "commit" : "commits"}`;
}

/** Returns the normalized owner and contributor pubkeys for a project. */
export function projectPeople(project: Project) {
  return [
    ...new Set(
      [project.owner, ...project.contributors]
        .filter(Boolean)
        .map(normalizePubkey),
    ),
  ];
}

/** Reports whether a repository snapshot contains any displayable content. */
export function snapshotHasContent(
  snapshot: ProjectRepoSnapshot | null | undefined,
) {
  return Boolean(
    snapshot &&
      (snapshot.latestCommit ||
        snapshot.commits.length > 0 ||
        snapshot.files.length > 0 ||
        snapshot.contributors.length > 0),
  );
}

/** Determines whether the active branch can open a pull request. */
export function createPullRequestAvailability(input: {
  activeBranch: string | null;
  defaultBranch: string;
  hasLocalCheckout: boolean;
  hasOpenPullRequest: boolean;
  localBranch: string | null | undefined;
  localHead: string | null | undefined;
  remoteHead: string | null | undefined;
}) {
  if (input.hasOpenPullRequest) {
    return {
      enabled: false,
      reason: "A pull request already exists for this branch.",
    };
  }
  if (!input.activeBranch || input.activeBranch === input.defaultBranch) {
    return {
      enabled: false,
      reason: "Select a feature branch to create a pull request.",
    };
  }
  if (!input.hasLocalCheckout) {
    return {
      enabled: false,
      reason: "Clone the repository before creating a pull request.",
    };
  }
  if (input.localBranch !== input.activeBranch) {
    return {
      enabled: false,
      reason: `Check out ${input.activeBranch} locally first.`,
    };
  }
  if (!input.localHead || input.localHead !== input.remoteHead) {
    return {
      enabled: false,
      reason: "Push this branch before creating a pull request.",
    };
  }
  return {
    enabled: true,
    reason: "Create a pull request from this branch.",
  };
}
