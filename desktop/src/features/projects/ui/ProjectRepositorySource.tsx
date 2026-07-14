import { ChevronDown, GitBranch, UploadCloud } from "lucide-react";

import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

/** Branch picker shared by the readme and files panel headers. */
export function RepositoryBranchDropdown({
  branch,
  branchOptions,
  compact,
  onBranchChange,
}: {
  branch: string;
  branchOptions: string[];
  /** Smaller trigger for inline headers. */
  compact?: boolean;
  onBranchChange: (branch: string) => void;
}) {
  const selectableBranches =
    branchOptions.length > 0 ? branchOptions : [branch];
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
              ? "h-6 max-w-full gap-1 px-1.5 font-mono text-xs font-medium"
              : "h-6 max-w-full gap-1.5 px-2 font-mono text-sm font-semibold"
          }
          size="sm"
          type="button"
          variant="ghost"
        >
          <GitBranch className="h-3 w-3 shrink-0 text-muted-foreground" />
          <span className="truncate">{branch}</span>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56">
        <DropdownMenuRadioGroup onValueChange={onBranchChange} value={branch}>
          {selectableBranches.map((option) => (
            <DropdownMenuRadioItem key={option} value={option}>
              <span className="truncate font-mono">{option}</span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Props for the compact branch + remote/local controls in panel headers. */
export type RepoSourceHeaderControls = {
  branch: string;
  branchOptions: string[];
  onBranchChange: (branch: string) => void;
  source: "remote" | "local";
  onSourceChange: (source: "remote" | "local") => void;
  localDisabled: boolean;
  localLabel: string;
  remoteLabel: string;
  /** Push of local commits, shown only when the local source can push. */
  canPush?: boolean;
  onPush?: () => void;
  pushDisabled?: boolean;
  pushPending?: boolean;
  pushTitle?: string;
};

/** Small pill toggle between the remote and local repository source. */
export function RepoSourceToggle({
  controls,
}: {
  controls: RepoSourceHeaderControls;
}) {
  const showPush =
    controls.source === "local" && controls.canPush && controls.onPush;
  return (
    <div className="flex shrink-0 items-center gap-1">
      {showPush ? (
        <Button
          aria-label={
            controls.pushPending
              ? "Pushing local commits"
              : "Push local commits"
          }
          className="h-6 w-6 shrink-0 rounded-full"
          disabled={controls.pushDisabled}
          onClick={controls.onPush}
          size="icon"
          title={controls.pushTitle ?? "Push local commits"}
          variant="default"
        >
          <UploadCloud className="h-3.5 w-3.5" />
        </Button>
      ) : null}
      <div className="flex shrink-0 items-center gap-0.5 border border-border/60 bg-muted/40 p-0.5">
        <Button
          className="h-6 px-2.5 text-xs"
          onClick={() => controls.onSourceChange("remote")}
          size="sm"
          variant={controls.source === "remote" ? "secondary" : "ghost"}
        >
          {controls.remoteLabel}
        </Button>
        <Button
          className="h-6 px-2.5 text-xs"
          disabled={controls.localDisabled}
          onClick={() => controls.onSourceChange("local")}
          size="sm"
          variant={controls.source === "local" ? "secondary" : "ghost"}
        >
          {controls.localLabel}
        </Button>
      </div>
    </div>
  );
}
