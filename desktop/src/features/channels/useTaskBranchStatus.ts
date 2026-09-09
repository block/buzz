import { useQuery } from "@tanstack/react-query";
import {
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
} from "lucide-react";
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
    ? GitBranch
    : pr.state === "MERGED"
      ? GitMerge
      : pr.state === "CLOSED"
        ? GitPullRequestClosed
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
      ? "text-purple-500"
      : pr.state === "CLOSED"
        ? "text-red-500"
        : pr.isDraft
          ? "text-muted-foreground"
          : "text-green-500";
  return { query, pr, Icon, label, color };
}
