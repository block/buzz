import { Ellipsis, Play, Square, Trash2 } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type MembersSidebarAgentControlsProps = {
  canBulkRemove: boolean;
  canBulkRespawn: boolean;
  canBulkStop: boolean;
  disabled: boolean;
  onRemoveAll: () => void;
  onRespawnAll: () => void;
  onStopAll: () => void;
};

export function MembersSidebarAgentControls({
  canBulkRemove,
  canBulkRespawn,
  canBulkStop,
  disabled,
  onRemoveAll,
  onRespawnAll,
  onStopAll,
}: MembersSidebarAgentControlsProps) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          className="ml-auto flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          data-testid="members-sidebar-agent-controls"
          type="button"
        >
          <Ellipsis className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem
          data-testid="members-sidebar-respawn-all"
          disabled={disabled || !canBulkRespawn}
          onClick={onRespawnAll}
        >
          <Play className="h-4 w-4" />
          Spawn or respawn all
        </DropdownMenuItem>
        <DropdownMenuItem
          data-testid="members-sidebar-stop-all"
          disabled={disabled || !canBulkStop}
          onClick={onStopAll}
        >
          <Square className="h-4 w-4" />
          Stop all
        </DropdownMenuItem>
        {canBulkRemove ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              data-testid="members-sidebar-remove-all"
              disabled={disabled}
              onClick={onRemoveAll}
            >
              <Trash2 className="h-4 w-4" />
              Remove all from channel
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
