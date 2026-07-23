import { GitCommit } from "lucide-react";
import { relativeTime } from "@/shared/lib/relative-time";
import type { CommitInfo } from "../git-client";

function CommitRow({ commit }: { commit: CommitInfo }) {
  const firstLine = commit.message.split("\n")[0];
  return (
    <div className="flex items-start gap-3 border-b border-black/10 px-3 py-2.5 text-sm text-black last:border-b-0 dark:border-white/10 dark:text-white">
      <GitCommit className="mt-0.5 h-4 w-4 shrink-0 text-black/50 dark:text-white/50" />
      <div className="min-w-0 flex-1">
        <p className="truncate font-medium">{firstLine}</p>
        <p className="mt-0.5 text-xs text-black/50 dark:text-white/50">
          {commit.author.name} committed {relativeTime(commit.author.timestamp)}
        </p>
      </div>
      <code className="shrink-0 self-center rounded bg-black/5 px-1.5 py-0.5 font-mono text-xs text-black/50 dark:bg-white/10 dark:text-white/50">
        {commit.oid.slice(0, 7)}
      </code>
    </div>
  );
}

export function RepoCommitsSection({
  commits,
  isLoading,
}: {
  commits: CommitInfo[] | undefined;
  isLoading: boolean;
}) {
  if (isLoading) {
    return (
      <div className="mt-8">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold text-black dark:text-white">
          <GitCommit className="h-4 w-4" />
          Recent commits
        </h2>
        <div className="rounded-lg border border-black/10 dark:border-white/10">
          {["sk-1", "sk-2", "sk-3"].map((key) => (
            <div
              key={key}
              className="flex items-center gap-3 border-b border-black/10 px-3 py-2.5 last:border-b-0 dark:border-white/10"
            >
              <div className="h-4 w-4 animate-pulse rounded bg-black/10 dark:bg-white/10" />
              <div className="flex-1 space-y-1">
                <div className="h-4 w-48 animate-pulse rounded bg-black/10 dark:bg-white/10" />
                <div className="h-3 w-32 animate-pulse rounded bg-black/10 dark:bg-white/10" />
              </div>
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (!commits || commits.length === 0) return null;

  return (
    <div className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold text-black dark:text-white">
        <GitCommit className="h-4 w-4" />
        Recent commits
      </h2>
      <div className="overflow-hidden rounded-lg border border-black/10 bg-white/50 dark:border-white/10 dark:bg-white/5">
        {commits.map((commit) => (
          <CommitRow key={commit.oid} commit={commit} />
        ))}
      </div>
    </div>
  );
}
