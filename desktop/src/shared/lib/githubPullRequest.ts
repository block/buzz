import { useQuery } from "@tanstack/react-query";

import { invokeTauri } from "@/shared/api/tauri";

export type GithubPullRequestInfo = {
  title: string;
  /** GitHub PR state: `open` or `closed` (merged PRs report `closed`). */
  state: string;
  merged: boolean;
  draft: boolean;
  additions: number;
  deletions: number;
  changedFiles: number;
  /** Source branch of the PR (`head.ref`). */
  headRef: string;
  /** Head commit sha — used to query check runs. */
  headSha: string;
  /** Issue-level comment count. */
  comments: number;
  /** Review (inline) comment count. */
  reviewComments: number;
};

export type GithubCheckSummary = {
  total: number;
  pending: number;
  failed: number;
  succeeded: number;
  /** Individual check runs for the expanded monitor view. */
  runs: Array<{ name: string; state: "pending" | "success" | "failure" }>;
};

export type GithubCommentState = {
  /**
   * Review threads still awaiting the PR author's reply. REST can't see the
   * "resolved" bit, so replying is what clears a thread from this count.
   */
  openThreads: number;
};

export type GithubPullRequestRef = {
  owner: string;
  repo: string;
  number: number;
};

/** Parse `github.com/{owner}/{repo}/pull/{number}` out of a PR link. */
export function parseGithubPullRequestRef(
  href: string,
): GithubPullRequestRef | null {
  let parsed: URL;
  try {
    parsed = new URL(href);
  } catch {
    return null;
  }
  if (parsed.hostname.toLowerCase().replace(/^www\./, "") !== "github.com") {
    return null;
  }
  const [owner, repo, resource, number] = parsed.pathname
    .split("/")
    .filter(Boolean);
  if (!owner || !repo || resource !== "pull" || !/^\d+$/.test(number ?? "")) {
    return null;
  }
  return { owner, repo, number: Number(number) };
}

export function githubPullRequestStatus(info: GithubPullRequestInfo) {
  if (info.merged) return "merged" as const;
  if (info.state === "closed") return "closed" as const;
  if (info.draft) return "draft" as const;
  return "open" as const;
}

export function useGithubPullRequestQuery(ref: GithubPullRequestRef | null) {
  return useQuery({
    enabled: ref !== null,
    queryKey: [
      "github-pull-request",
      ref?.owner ?? "",
      ref?.repo ?? "",
      ref?.number ?? 0,
    ],
    queryFn: async () =>
      (await invokeTauri<GithubPullRequestInfo | null>(
        "fetch_github_pull_request",
        {
          owner: ref?.owner ?? "",
          repo: ref?.repo ?? "",
          number: ref?.number ?? 0,
        },
      )) ?? null,
    staleTime: 60_000,
    // The work panel doubles as a PR monitor — keep state and comment
    // counts fresh while mounted.
    refetchInterval: 60_000,
    retry: 1,
  });
}

export function useGithubCheckSummaryQuery(
  ref: GithubPullRequestRef | null,
  sha: string | null | undefined,
) {
  return useQuery({
    enabled: ref !== null && Boolean(sha),
    queryKey: [
      "github-check-summary",
      ref?.owner ?? "",
      ref?.repo ?? "",
      sha ?? "",
    ],
    queryFn: async () =>
      (await invokeTauri<GithubCheckSummary | null>(
        "fetch_github_check_summary",
        {
          owner: ref?.owner ?? "",
          repo: ref?.repo ?? "",
          sha: sha ?? "",
        },
      )) ?? null,
    staleTime: 30_000,
    // CI flips fast while runs execute; poll while the panel is mounted.
    refetchInterval: 45_000,
    retry: 1,
  });
}

/**
 * Discover the PR for a branch via the project's git remote — the fallback
 * when no PR link was ever posted in the chat. Polls slowly so a PR opened
 * mid-conversation surfaces on its own.
 */
export function useGithubPrForBranchQuery(
  projectPath: string | null | undefined,
  branch: string | null | undefined,
) {
  return useQuery({
    enabled: Boolean(projectPath) && Boolean(branch),
    queryKey: ["github-pr-for-branch", projectPath ?? "", branch ?? ""],
    queryFn: async () =>
      (await invokeTauri<string | null>("find_github_pr_for_branch", {
        projectPath: projectPath ?? "",
        branch: branch ?? "",
      })) ?? null,
    staleTime: 60_000,
    refetchInterval: 90_000,
    retry: 1,
  });
}

export function useGithubCommentStateQuery(ref: GithubPullRequestRef | null) {
  return useQuery({
    enabled: ref !== null,
    queryKey: [
      "github-pr-comment-state",
      ref?.owner ?? "",
      ref?.repo ?? "",
      ref?.number ?? 0,
    ],
    queryFn: async () =>
      (await invokeTauri<GithubCommentState | null>(
        "fetch_github_pr_comment_state",
        {
          owner: ref?.owner ?? "",
          repo: ref?.repo ?? "",
          number: ref?.number ?? 0,
        },
      )) ?? null,
    staleTime: 30_000,
    refetchInterval: 60_000,
    retry: 1,
  });
}
