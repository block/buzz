import {
  ChevronDown,
  Cloud,
  DownloadCloud,
  GitBranch,
  HardDrive,
  Loader2,
  Plus,
  RefreshCw,
  Tag,
  Trash2,
  UploadCloud,
} from "lucide-react";

import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { PROJECT_PANEL_ACTION_BUTTON_CLASS } from "./projectPanelStyles";

/** Branch picker shared by the readme and files panel headers. */
export function RepositoryBranchDropdown({
  branch,
  branchOptions,
  selectedTag,
  tagOptions = [],
  compact,
  createBranchDisabled,
  createBranchTitle,
  deleteBranchDisabled,
  deleteBranchTitle,
  onBranchChange,
  onTagChange,
  onCreateBranch,
  onDeleteBranch,
}: {
  branch: string;
  branchOptions: string[];
  selectedTag?: string | null;
  tagOptions?: Array<{ name: string; commit: string }>;
  /** Smaller trigger for inline headers. */
  compact?: boolean;
  createBranchDisabled?: boolean;
  createBranchTitle?: string;
  deleteBranchDisabled?: boolean;
  deleteBranchTitle?: string;
  onBranchChange: (branch: string) => void;
  onTagChange?: (tag: string) => void;
  onCreateBranch?: () => void;
  onDeleteBranch?: () => void;
}) {
  const selectableBranches =
    branchOptions.length > 0 ? branchOptions : [branch];
  const selectedValue = selectedTag ? `tag:${selectedTag}` : `branch:${branch}`;
  const RefIcon = selectedTag ? Tag : GitBranch;
  if (!branch) {
    return (
      <span className="truncate font-mono text-sm font-semibold text-foreground">
        —
      </span>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          className={
            compact
              ? "h-7 max-w-full gap-1.5 rounded-md px-3 font-mono text-sm font-medium hover:border-input"
              : "h-6 max-w-full gap-1.5 px-2 font-mono text-sm font-semibold hover:border-input"
          }
          size="sm"
          type="button"
          variant="outline"
        >
          <RefIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="truncate">{selectedTag ?? branch}</span>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56">
        <DropdownMenuRadioGroup
          onValueChange={(value) => {
            if (value.startsWith("tag:")) {
              onTagChange?.(value.slice("tag:".length));
            } else {
              onBranchChange(value.slice("branch:".length));
            }
          }}
          value={selectedValue}
        >
          <DropdownMenuLabel>Branches</DropdownMenuLabel>
          {selectableBranches.map((option) => (
            <DropdownMenuRadioItem key={option} value={`branch:${option}`}>
              <GitBranch className="mr-1.5 h-3.5 w-3.5 text-muted-foreground" />
              <span className="truncate font-mono">{option}</span>
            </DropdownMenuRadioItem>
          ))}
          {tagOptions.length > 0 ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>Tags</DropdownMenuLabel>
              {tagOptions.map((option) => (
                <DropdownMenuRadioItem
                  key={option.name}
                  value={`tag:${option.name}`}
                >
                  <Tag className="mr-1.5 h-3.5 w-3.5 text-muted-foreground" />
                  <span className="truncate font-mono">{option.name}</span>
                  <span className="ml-auto font-mono text-xs text-muted-foreground">
                    {option.commit.slice(0, 7)}
                  </span>
                </DropdownMenuRadioItem>
              ))}
            </>
          ) : null}
        </DropdownMenuRadioGroup>
        {!selectedTag && (onCreateBranch || onDeleteBranch) ? (
          <>
            <DropdownMenuSeparator />
            {onCreateBranch ? (
              <>
                <DropdownMenuItem
                  data-testid="project-create-branch"
                  disabled={createBranchDisabled}
                  onSelect={onCreateBranch}
                  title={createBranchTitle}
                >
                  <Plus className="h-4 w-4" />
                  Create branch…
                </DropdownMenuItem>
                {createBranchDisabled && createBranchTitle ? (
                  <p className="max-w-56 px-2 py-1 text-xs text-muted-foreground">
                    {createBranchTitle}
                  </p>
                ) : null}
              </>
            ) : null}
            {onDeleteBranch ? (
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                data-testid="project-delete-branch"
                disabled={deleteBranchDisabled}
                onSelect={onDeleteBranch}
                title={deleteBranchTitle}
              >
                <Trash2 className="h-4 w-4" />
                Delete {branch}
              </DropdownMenuItem>
            ) : null}
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Props for the compact branch + remote/local controls in panel headers. */
export type RepoSourceHeaderControls = {
  branch: string;
  branchOptions: string[];
  selectedTag?: string | null;
  tagOptions?: Array<{ name: string; commit: string }>;
  onBranchChange: (branch: string) => void;
  onTagChange?: (tag: string) => void;
  onCreateBranch?: () => void;
  createBranchDisabled?: boolean;
  createBranchTitle?: string;
  onDeleteBranch?: () => void;
  deleteBranchDisabled?: boolean;
  deleteBranchTitle?: string;
  source: "remote" | "local";
  onSourceChange: (source: "remote" | "local") => void;
  localDisabled: boolean;
  localLabel: string;
  remoteLabel: string;
  /** Clones the repository when no local checkout is available. */
  onCloneLocal?: () => void;
  clonePending?: boolean;
  /** Push of local commits, available when the local checkout is ahead. */
  canPush?: boolean;
  onPush?: () => void;
  pushDisabled?: boolean;
  pushPending?: boolean;
  pushTitle?: string;
  /** Fast-forward pull, available when the local checkout is behind. */
  canPull?: boolean;
  onPull?: () => void;
  pullDisabled?: boolean;
  pullPending?: boolean;
  pullTitle?: string;
  /** Commits the local checkout is ahead/behind the remote branch. */
  aheadCount?: number | null;
  behindCount?: number | null;
  /** Manual sync-status refresh (runs a git fetch under the hood). */
  onFetch?: () => void;
  fetchPending?: boolean;
  fetchTitle?: string;
};

/** Compact dropdown picking the repository source (remote or local). */
export function RepoSourceDropdown({
  controls,
}: {
  controls: RepoSourceHeaderControls;
}) {
  const isLocal = controls.source === "local";
  const cloneLocal = controls.localDisabled && controls.onCloneLocal;
  const SourceIcon = isLocal ? HardDrive : Cloud;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          className="h-7 max-w-full shrink-0 gap-1.5 rounded-md px-3 text-sm font-medium hover:border-input"
          size="sm"
          type="button"
          variant="outline"
        >
          <SourceIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
          <span className="truncate">
            {isLocal ? controls.localLabel : controls.remoteLabel}
          </span>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-40">
        <DropdownMenuRadioGroup
          onValueChange={(value) =>
            controls.onSourceChange(value === "local" ? "local" : "remote")
          }
          value={controls.source}
        >
          <DropdownMenuRadioItem value="remote">
            <Cloud className="mr-1.5 h-3.5 w-3.5 text-muted-foreground" />
            {controls.remoteLabel}
          </DropdownMenuRadioItem>
          {!cloneLocal ? (
            <DropdownMenuRadioItem
              disabled={controls.localDisabled}
              value="local"
            >
              <HardDrive className="mr-1.5 h-3.5 w-3.5 text-muted-foreground" />
              {controls.localLabel}
            </DropdownMenuRadioItem>
          ) : null}
        </DropdownMenuRadioGroup>
        {cloneLocal ? (
          <DropdownMenuItem
            className="group"
            disabled={controls.clonePending}
            onSelect={controls.onCloneLocal}
          >
            {controls.clonePending ? (
              <Loader2 className="animate-spin text-muted-foreground" />
            ) : (
              <DownloadCloud className="text-muted-foreground" />
            )}
            <span className="text-muted-foreground">{controls.localLabel}</span>
            <span className="ml-auto rounded-md border border-input/60 bg-background px-2 py-0.5 text-xs font-medium text-foreground shadow-xs group-focus:border-input">
              {controls.clonePending ? "Cloning…" : "Clone"}
            </span>
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Always-visible contextual sync button: Pull when the local checkout is
 * behind, Push when ahead, otherwise Fetch to refresh the status. */
export function RepoSyncActionButton({
  controls,
}: {
  controls: RepoSourceHeaderControls;
}) {
  const pull = controls.canPull && controls.onPull;
  const push = controls.canPush && controls.onPush;

  if (pull) {
    const count = controls.behindCount ?? 0;
    return (
      <Button
        className={PROJECT_PANEL_ACTION_BUTTON_CLASS}
        disabled={controls.pullDisabled}
        onClick={controls.onPull}
        size="sm"
        title={controls.pullTitle ?? "Pull remote commits"}
        variant="ghost"
      >
        {controls.pullPending ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <DownloadCloud className="h-4 w-4" />
        )}
        Pull{count > 0 ? ` ${count}` : ""}
      </Button>
    );
  }

  if (push) {
    return (
      <Button
        className={PROJECT_PANEL_ACTION_BUTTON_CLASS}
        disabled={controls.pushDisabled}
        onClick={controls.onPush}
        size="sm"
        title={controls.pushTitle ?? "Push local commits"}
        variant="ghost"
      >
        {controls.pushPending ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <UploadCloud className="h-4 w-4" />
        )}
        Push
      </Button>
    );
  }

  if (!controls.onFetch) return null;
  return (
    <Button
      className={PROJECT_PANEL_ACTION_BUTTON_CLASS}
      disabled={controls.fetchPending}
      onClick={controls.onFetch}
      size="sm"
      title={controls.fetchTitle ?? "Check for remote changes"}
      variant="ghost"
    >
      {controls.fetchPending ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : (
        <RefreshCw className="h-4 w-4" />
      )}
      Fetch
    </Button>
  );
}
