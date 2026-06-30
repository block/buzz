import {
  Check,
  ChevronDown,
  Copy,
  GitBranch,
  GitFork,
  UploadCloud,
} from "lucide-react";
import * as React from "react";

import type { ProjectRepoSyncStatus } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function CloneUrlRow({ url }: { url: string }) {
  const [copied, setCopied] = React.useState(false);

  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [url]);

  return (
    <div className="flex min-w-0 items-center gap-2">
      <GitFork className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <code className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
        {url}
      </code>
      <Button
        className="h-6 w-6 shrink-0"
        onClick={handleCopy}
        size="icon"
        variant="ghost"
      >
        {copied ? (
          <Check className="h-4 w-4 text-green-500" />
        ) : (
          <Copy className="h-4 w-4" />
        )}
      </Button>
    </div>
  );
}

export function RepositorySourceCard({
  branch,
  branchOptions,
  cloneUrls,
  localDisabled,
  localLabel,
  onBranchChange,
  onSourceChange,
  remoteLabel,
  source,
}: {
  branch: string;
  branchOptions: string[];
  cloneUrls: string[];
  localDisabled: boolean;
  localLabel: string;
  onBranchChange: (branch: string) => void;
  onSourceChange: (source: "remote" | "local") => void;
  remoteLabel: string;
  source: "remote" | "local";
}) {
  if (cloneUrls.length === 0 && !branch) return null;
  const selectableBranches =
    branchOptions.length > 0 ? branchOptions : [branch];

  return (
    <Card className="border-border/50 bg-card/60 p-4 shadow-none">
      <div className="flex min-w-0 flex-col gap-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <div className="flex min-w-0 items-center gap-2">
            <GitBranch className="h-3.5 w-3.5 text-muted-foreground" />
            {branch ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    className="h-6 max-w-full gap-1.5 px-2 font-mono text-sm font-semibold"
                    size="sm"
                    type="button"
                    variant="ghost"
                  >
                    <span className="truncate">{branch || "—"}</span>
                    <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="min-w-56">
                  <DropdownMenuRadioGroup
                    onValueChange={onBranchChange}
                    value={branch}
                  >
                    {selectableBranches.map((option) => (
                      <DropdownMenuRadioItem key={option} value={option}>
                        <span className="truncate font-mono">{option}</span>
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
            ) : (
              <span className="truncate font-mono text-sm font-semibold text-foreground">
                {branch || "—"}
              </span>
            )}
          </div>
          <div className="ml-auto flex w-fit items-center gap-1 rounded-xl border border-border/50 bg-background/55 p-1">
            <Button
              className="h-7 px-3"
              onClick={() => onSourceChange("remote")}
              size="sm"
              variant={source === "remote" ? "secondary" : "ghost"}
            >
              {remoteLabel}
            </Button>
            <Button
              className="h-7 px-3"
              disabled={localDisabled}
              onClick={() => onSourceChange("local")}
              size="sm"
              variant={source === "local" ? "secondary" : "ghost"}
            >
              {localLabel}
            </Button>
          </div>
        </div>
        <div className="min-w-0 flex-1">
          {cloneUrls.length > 0 ? (
            cloneUrls.map((url) => <CloneUrlRow key={url} url={url} />)
          ) : (
            <div className="text-sm text-muted-foreground">
              No clone URL published yet.
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}

function formatSyncStatus(status: ProjectRepoSyncStatus | null | undefined) {
  if (!status) return "Checking repository status…";
  if (!status.localPath) return "No local checkout found.";
  if (status.aheadCount > 0 && status.behindCount > 0) {
    return `Local and remote have diverged by ${pluralize(status.aheadCount, "local commit")} and ${pluralize(status.behindCount, "remote commit")}.`;
  }
  if (status.aheadCount > 0) {
    return `Local is ahead by ${pluralize(status.aheadCount, "commit")}.`;
  }
  if (status.behindCount > 0) {
    return `Remote is ahead by ${pluralize(status.behindCount, "commit")}.`;
  }
  if (status.localHead && status.remoteHead)
    return "Local and remote are in sync.";
  if (status.localHead && !status.remoteHead) {
    return "Local has commits that are not pushed yet.";
  }
  return "No commits are available yet.";
}

export function RepositorySyncStatusCard({
  isLoading,
  onPush,
  pushDisabled,
  pushPending,
  status,
}: {
  isLoading: boolean;
  onPush: () => void;
  pushDisabled: boolean;
  pushPending: boolean;
  status: ProjectRepoSyncStatus | null | undefined;
}) {
  const localLabel = status?.localShortHead
    ? `${status.localBranch ?? "local"} @ ${status.localShortHead}`
    : "No local commit";
  const remoteLabel = status?.remoteShortHead
    ? `${status.remoteBranch ?? "remote"} @ ${status.remoteShortHead}`
    : "No pushed commit";
  const dirtyLabel =
    status?.hasUncommittedChanges || status?.hasUntrackedFiles
      ? "Uncommitted changes present"
      : "Working tree clean";

  return (
    <Card className="border-border/50 bg-card/60 p-4 shadow-none">
      <div className="flex min-w-0 flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
            <span>
              <span className="font-medium text-foreground">Remote:</span>{" "}
              {remoteLabel}
            </span>
            <span>
              <span className="font-medium text-foreground">Local:</span>{" "}
              {localLabel}
            </span>
            <span>{dirtyLabel}</span>
          </div>
          <p className="text-sm text-muted-foreground">
            {isLoading
              ? "Checking repository status…"
              : formatSyncStatus(status)}
          </p>
          {status?.localPath ? (
            <code className="block truncate text-xs text-muted-foreground">
              {status.localPath}
            </code>
          ) : null}
        </div>
        <Button
          className="shrink-0 gap-1.5"
          disabled={pushDisabled}
          onClick={onPush}
          size="sm"
          title={status?.pushBlockReason ?? undefined}
          variant={status?.canPush ? "default" : "outline"}
        >
          <UploadCloud className="h-4 w-4" />
          {pushPending ? "Pushing…" : "Push"}
        </Button>
      </div>
      {status?.pushBlockReason && !status.canPush ? (
        <p className="mt-2 text-xs text-muted-foreground">
          {status.pushBlockReason}
        </p>
      ) : null}
    </Card>
  );
}

export function RepositorySourceNotice({
  path,
  source,
}: {
  path?: string | null;
  source: "local" | "remote";
}) {
  return (
    <div className="rounded-xl border border-amber-500/25 bg-amber-500/8 px-4 py-3 text-xs text-muted-foreground">
      {source === "local" ? (
        <>
          Showing local checkout data
          {path ? (
            <>
              {" "}
              from <code className="text-foreground">{path}</code>
            </>
          ) : null}
          . These commits may not be pushed yet.
        </>
      ) : (
        "Showing remote data from the project clone URL."
      )}
    </div>
  );
}
