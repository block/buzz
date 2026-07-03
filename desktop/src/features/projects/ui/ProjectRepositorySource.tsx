import {
  Check,
  ChevronDown,
  Copy,
  GitBranch,
  GitFork,
  HardDrive,
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

function RepositoryPathRow({
  action,
  path,
  title,
  type,
}: {
  action?: React.ReactNode;
  path: string;
  title?: string;
  type: "remote" | "local";
}) {
  const [copied, setCopied] = React.useState(false);

  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(path).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [path]);
  const Icon = type === "local" ? HardDrive : GitFork;

  return (
    <div className="flex min-w-0 items-center gap-2" title={title}>
      <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <code className="min-w-0 max-w-full truncate text-xs text-muted-foreground">
          {path}
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
      {action}
    </div>
  );
}

export function RepositorySourceCard({
  branch,
  branchOptions,
  cloneUrls,
  localDisabled,
  localLabel,
  localPath,
  onBranchChange,
  onPush,
  onSourceChange,
  pushDisabled,
  pushPending,
  remoteLabel,
  source,
  status,
}: {
  branch: string;
  branchOptions: string[];
  cloneUrls: string[];
  localDisabled: boolean;
  localLabel: string;
  localPath?: string | null;
  onBranchChange: (branch: string) => void;
  onPush: () => void;
  onSourceChange: (source: "remote" | "local") => void;
  pushDisabled: boolean;
  pushPending: boolean;
  remoteLabel: string;
  source: "remote" | "local";
  status: ProjectRepoSyncStatus | null | undefined;
}) {
  if (cloneUrls.length === 0 && !branch) return null;
  const selectableBranches =
    branchOptions.length > 0 ? branchOptions : [branch];
  const showLocalPath = source === "local" && localPath;
  const localRefLabel = status?.localShortHead
    ? `${status.localBranch ?? "local"} @ ${status.localShortHead}`
    : null;
  const showPushButton = source === "local" && status?.canPush;
  const pushAction = showPushButton ? (
    <Button
      aria-label={pushPending ? "Pushing local commits" : "Push local commits"}
      className="h-6 w-6 shrink-0 rounded-full"
      disabled={pushDisabled}
      onClick={onPush}
      size="icon"
      title={status?.pushBlockReason ?? "Push local commits"}
      variant="default"
    >
      <UploadCloud className="h-3.5 w-3.5" />
    </Button>
  ) : null;

  return (
    <Card className="relative mt-8 border-border/50 bg-card/60 px-4 py-3 shadow-none">
      <div className="-translate-x-1/2 -translate-y-1/2 absolute top-0 left-1/2 flex w-fit items-center gap-1 rounded-full border border-border/50 bg-card px-1 py-1 shadow-sm">
        <Button
          className="h-7 rounded-full px-3"
          onClick={() => onSourceChange("remote")}
          size="sm"
          variant={source === "remote" ? "secondary" : "ghost"}
        >
          {remoteLabel}
        </Button>
        <Button
          className="h-7 rounded-full px-3"
          disabled={localDisabled}
          onClick={() => onSourceChange("local")}
          size="sm"
          variant={source === "local" ? "secondary" : "ghost"}
        >
          {localLabel}
        </Button>
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="flex min-w-0 items-center justify-between gap-2">
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
        </div>
        <div className="min-w-0 flex-1">
          {showLocalPath ? (
            <RepositoryPathRow
              action={pushAction}
              path={localPath}
              title={
                localRefLabel ? `Local checkout: ${localRefLabel}` : undefined
              }
              type="local"
            />
          ) : cloneUrls.length > 0 ? (
            cloneUrls.map((url) => (
              <RepositoryPathRow key={url} path={url} type="remote" />
            ))
          ) : (
            <div className="text-sm text-muted-foreground">
              {source === "local"
                ? "No local checkout found."
                : "No clone URL published yet."}
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}
