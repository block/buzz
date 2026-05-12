import { GitCommit } from "lucide-react";
import type { CommitInfo } from "../git-client";

function relativeTime(unix: number): string {
  const now = Date.now();
  const diff = now - unix * 1000;
  const seconds = Math.floor(diff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (days > 30) {
    const months = Math.floor(days / 30);
    return months === 1 ? "1 month ago" : `${months} months ago`;
  }
  if (days > 0) return days === 1 ? "1 day ago" : `${days} days ago`;
  if (hours > 0) return hours === 1 ? "1 hour ago" : `${hours} hours ago`;
  if (minutes > 0)
    return minutes === 1 ? "1 minute ago" : `${minutes} minutes ago`;
  return "just now";
}

function CommitRow({ commit }: { commit: CommitInfo }) {
  const firstLine = commit.message.split("\n")[0];
  return (
    <div className="flex items-start gap-3 border-b border-border px-3 py-2.5 text-sm last:border-b-0">
      <GitCommit className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <p className="truncate font-medium">{firstLine}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">
          {commit.author.name} committed {relativeTime(commit.author.timestamp)}
        </p>
      </div>
      <code className="shrink-0 self-center rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">
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
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <GitCommit className="h-4 w-4" />
          Recent commits
        </h2>
        <div className="rounded-lg border border-border">
          {["sk-1", "sk-2", "sk-3"].map((key) => (
            <div
              key={key}
              className="flex items-center gap-3 border-b border-border px-3 py-2.5 last:border-b-0"
            >
              <div className="h-4 w-4 animate-pulse rounded bg-muted" />
              <div className="flex-1 space-y-1">
                <div className="h-4 w-48 animate-pulse rounded bg-muted" />
                <div className="h-3 w-32 animate-pulse rounded bg-muted" />
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
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
        <GitCommit className="h-4 w-4" />
        Recent commits
      </h2>
      <div className="overflow-hidden rounded-lg border border-border">
        {commits.map((commit) => (
          <CommitRow key={commit.oid} commit={commit} />
        ))}
      </div>
    </div>
  );
}
