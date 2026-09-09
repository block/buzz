import { useQuery } from "@tanstack/react-query";
import {
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
} from "lucide-react";
import { TaskBranchGlyph } from "@/features/channels/ui/TaskBranchGlyph";
import {
  githubRepositoryUrl,
  type ChannelBackedTask,
} from "@/features/channels/lib/channelBackedTask";
import { invokeTauri } from "@/shared/api/tauri";

type PullRequest = {
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft: boolean;
};

export function useTaskBranchStatus(
  branch: NonNullable<ChannelBackedTask["branch"]>,
) {
  const repository = githubRepositoryUrl(branch.repository);
  const query = useQuery({
    queryKey: ["task-github", repository, branch.name, "review"],
    enabled: Boolean(repository),
    queryFn: () =>
      invokeTauri<PullRequest[]>("get_task_github", {
        repository: repository?.replace("https://github.com/", ""),
        branch: branch.name,
        view: "review",
      }),
    staleTime: 60_000,
    retry: false,
  });
  const pr =
    query.data?.find((item) => item.state === "OPEN") ?? query.data?.[0];
  const Icon = !pr
    ? TaskBranchGlyph
    : pr.state === "MERGED"
      ? GitMerge
      : pr.state === "CLOSED"
        ? GitPullRequestClosed
        : pr.isDraft
          ? GitPullRequestDraft
          : GitPullRequest;
  const label = !pr
    ? "Branch"
    : pr.state === "MERGED"
      ? "Merged PR"
      : pr.state === "CLOSED"
        ? "Closed PR"
        : pr.isDraft
          ? "Draft PR"
          : "Open PR";
  const color = !pr
    ? "text-muted-foreground"
    : pr.state === "MERGED"
      ? "text-purple-600 dark:text-purple-400"
      : pr.state === "CLOSED"
        ? "text-red-600 dark:text-red-400"
        : pr.isDraft
          ? "text-muted-foreground"
          : "text-green-600 dark:text-green-400";
  return { query, pr, Icon, label, color };
}
